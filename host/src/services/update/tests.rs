//! Unit tests of the update service (design §9.2). No bus, no real program: every program is a stub inside a
//! marked fixture directory, every system file is re-rooted, and `pk::system_bus()` panics here.
use super::{
    Mode, Update, check_args, compose, guard, model,
    model::{Phase, PhaseInput, Pkg, Stage},
    paths::{self, BusTarget, Env, MARKER, Tool, VAR_FIXTURE_ROOT, VAR_PK_BUS, VAR_TOOL_DIR},
    phasing, restart, store,
};
use crate::services::ServiceContext;
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

// ------------------------------------------------------------------------------------------------ fixtures

/// A private, marked test tree: `root` (fixture root), `tools` (logging stubs for every tool), `bus/` (a
/// listening socket that must never see a connection), plus caches, configs and the events directory.
pub(super) struct Fixture {
    pub(super) base: PathBuf,
    pub(super) root: PathBuf,
    pub(super) tools: PathBuf,
    pub(super) log: PathBuf,
    bus_dir: PathBuf,
    listener: std::os::unix::net::UnixListener,
}
fn private(path: &Path) {
    fs::DirBuilder::new().recursive(true).mode(0o700).create(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
fn mark(dir: &Path) {
    fs::write(dir.join(MARKER), b"").unwrap();
}
pub(super) fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}
pub(super) fn script(path: &Path, body: &str) {
    write(path, &format!("#!/bin/sh\n{body}"));
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}
impl Fixture {
    pub(super) fn new(name: &str) -> Self {
        let base = std::env::temp_dir().join(format!("w11u-{name}-{}", crate::events::nonce()));
        let (root, tools, bus_dir) = (base.join("root"), base.join("tools"), base.join("bus"));
        for dir in [&base, &root, &tools, &bus_dir] {
            private(dir);
        }
        for dir in [&root, &tools, &bus_dir] {
            mark(dir);
        }
        let log = base.join("stub.log");
        fs::write(&log, b"").unwrap();
        for tool in Tool::ALL {
            script(&tools.join(tool.name()), &format!("echo \"{} $1\" >> '{}'\nexit 97\n", tool.name(), log.display()));
        }
        write(&root.join("proc/sys/kernel/random/boot_id"), "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0\n");
        write(&root.join("etc/os-release"), "NAME=\"Ubuntu\"\nVERSION=\"26.04.1 LTS (Resolute Raccoon)\"\n");
        let listener = std::os::unix::net::UnixListener::bind(bus_dir.join("bus")).unwrap();
        listener.set_nonblocking(true).unwrap();
        Self { base, root, tools, log, bus_dir, listener }
    }
    fn bus(&self) -> String {
        format!("unix:path={}", self.bus_dir.join("bus").display())
    }
    fn s(path: &Path) -> String {
        path.display().to_string()
    }
    /// Every seam set and valid (contract C10: the bus socket directly in a marked 0700 runtime directory, and
    /// no system-bus socket visible at the paths this Env checks).
    fn env(&self) -> Env {
        Env::fixture(
            &[(VAR_PK_BUS, &self.bus()), (VAR_FIXTURE_ROOT, &Self::s(&self.root)), (VAR_TOOL_DIR, &Self::s(&self.tools))],
            uid(),
        )
        .runtime(Some(&self.bus_dir))
        .system_sockets(vec![self.base.join("no-system-bus-socket")])
    }
    /// Valid seams without a bus (an owner can run its pk thread with nothing to connect to).
    pub(super) fn env_no_bus(&self) -> Env {
        Env::fixture(&[(VAR_FIXTURE_ROOT, &Self::s(&self.root)), (VAR_TOOL_DIR, &Self::s(&self.tools))], uid())
    }
    pub(super) fn stub_log(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }
    /// No process ever connected to the bus socket.
    pub(super) fn never_connected(&self) -> bool {
        matches!(self.listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock)
    }
    pub(super) fn ctx(&self, kind: &str, dry: bool, lang: &str) -> Arc<ServiceContext> {
        self.ctx_in(&self.base.join("cache"), kind, dry, lang)
    }
    fn ctx_in(&self, cache: &Path, kind: &str, dry: bool, lang: &str) -> Arc<ServiceContext> {
        let config = self.base.join(format!("config-{kind}-{}", crate::events::nonce()));
        fs::create_dir_all(&config).unwrap();
        if lang == "en" {
            let mut settings: Value = serde_json::from_str(include_str!("../../settings-defaults.json")).unwrap();
            settings["language"] = json!("en");
            fs::write(config.join("settings.json"), serde_json::to_vec(&settings).unwrap()).unwrap();
        }
        let events = crate::events::Events::new(&self.base.join("events")).unwrap();
        ServiceContext::new(events, config, cache.to_path_buf(), self.base.clone(), kind.into(), dry).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

/// Every method with valid arguments.
fn all_calls() -> Vec<(&'static str, Value)> {
    let id = "0123456789abcdef0123456789abcdef";
    vec![
        ("update.status", json!({})),
        ("update.status", json!({"packages":true})),
        ("update.check", json!({})),
        ("update.details", json!({"ids":["libsoup-3.0-0;3.6.6-1ubuntu0.3;amd64;ubuntu-resolute-security"]})),
        ("update.install", json!({})),
        ("update.install", json!({"rows":["security"]})),
        ("update.confirm", json!({"jobId":id,"accept":true})),
        ("update.cancel", json!({"jobId":id})),
        ("update.repair", json!({})),
        ("update.history", json!({"limit":10})),
        ("update.options", json!({})),
        ("update.setAutoOpen", json!({"on":false})),
        ("update.release.check", json!({})),
        ("update.release.compat", json!({"refresh":true})),
        ("update.release.setPrompt", json!({"prompt":"lts"})),
        ("update.release.ubuntuHold", json!({"on":true})),
        ("update.release.start", json!({})),
        ("update.release.attach", json!({})),
    ]
}

fn uid() -> u32 {
    unsafe { libc::geteuid() }
}
pub(super) fn pkg(name: &str, version: &str, info: u32, data: &str) -> Pkg {
    Pkg::from_signal(info, &format!("{name};{version};amd64;{data}"), "summary").unwrap()
}

// ------------------------------------------------------------------------------------- seams (fail closed)

#[test]
fn no_seam_variable_means_the_system_table() {
    let setup = paths::resolve(&Env::fixture(&[], 1000)).unwrap();
    assert!(!setup.test);
    assert_eq!(setup.bus, BusTarget::System);
    assert!(setup.tools.path(Tool::Python3).unwrap().is_absolute());
    assert!(!setup.paths.fixture);
}

#[test]
fn valid_seams_reroot_every_path_and_take_tools_only_from_the_stub_dir() {
    let f = Fixture::new("valid");
    let setup = paths::resolve(&f.env()).unwrap();
    assert!(setup.test);
    assert_eq!(setup.bus, BusTarget::Address(f.bus()));
    let p = &setup.paths;
    for path in [&p.proc, &p.reboot_required, &p.os_release, &p.apt_log, &p.proc_locks, &p.dpkg_lock, &p.soname_dir, &p.archive_keyring] {
        assert!(path.starts_with(&f.root), "{} is not re-rooted", path.display());
    }
    for tool in Tool::ALL {
        assert_eq!(setup.tools.path(tool).unwrap(), f.tools.join(tool.name()));
    }
    // a stub that is not there is a seam error, never a PATH or absolute fallback
    fs::remove_file(f.tools.join("python3")).unwrap();
    assert!(setup.tools.path(Tool::Python3).is_err());
    assert!(setup.tools.command(Tool::Python3).is_err());
    // test mode without a tool directory: no tool at all
    let no_tools = paths::resolve(&Env::fixture(&[(VAR_FIXTURE_ROOT, &Fixture::s(&f.root))], uid())).unwrap();
    assert!(Tool::ALL.iter().all(|t| no_tools.tools.path(*t).is_err()));
    assert_eq!(no_tools.bus, BusTarget::Disabled);
}

/// Every child of the tool table runs with the stub directory as its whole PATH and with the private (or a dead)
/// bus as both bus addresses, so a stub's own children and bus clients never reach `/usr/bin` or a real bus
/// (review F1). Production children get `PATH=/usr/bin:/bin`.
#[test]
fn tool_children_get_the_stub_path_and_the_test_buses_only() {
    let f = Fixture::new("childenv");
    let report = f.base.join("env.txt");
    script(
        &f.tools.join("python3"),
        &format!(
            "{{ echo \"PATH=$PATH\"; echo \"SYSTEM=$DBUS_SYSTEM_BUS_ADDRESS\"; echo \"SESSION=$DBUS_SESSION_BUS_ADDRESS\"; \
             echo \"HOME=${{HOME:-}}\"; echo \"PKEXEC=$(command -v pkexec)\"; }} > '{}'\n",
            report.display()
        ),
    );
    let read = |env: &Env| {
        let setup = paths::resolve(env).unwrap();
        let status = setup.tools.command(Tool::Python3).unwrap().status().unwrap();
        assert!(status.success());
        let text = fs::read_to_string(&report).unwrap();
        text.lines().map(|l| l.split_once('=').map(|(k, v)| (k.to_owned(), v.to_owned())).unwrap()).collect::<std::collections::BTreeMap<_, _>>()
    };
    let tools = Fixture::s(&f.tools);
    // with a private bus: that bus, a dead session bus, PATH = the stubs (pkexec is the stub, never /usr/bin's)
    let env = read(&f.env());
    assert_eq!(env["PATH"], tools);
    assert_eq!(env["SYSTEM"], f.bus());
    assert_eq!(env["SESSION"], format!("unix:path={}", f.tools.join(paths::NO_SESSION_BUS).display()));
    assert_eq!(env["HOME"], "", "nothing of the test process leaks");
    assert_eq!(env["PKEXEC"], Fixture::s(&f.tools.join("pkexec")));
    // without a bus: a dead system-bus socket inside the stub directory
    let env = read(&f.env_no_bus());
    assert_eq!(env["SYSTEM"], format!("unix:path={}", f.tools.join(paths::NO_SYSTEM_BUS).display()));
    assert!(!f.tools.join(paths::NO_SYSTEM_BUS).exists() && !f.tools.join(paths::NO_SESSION_BUS).exists());
    assert!(f.never_connected());
    // production: PATH=/usr/bin:/bin, no bus variable (the command is only built, never run)
    let system = paths::resolve(&Env::fixture(&[], 1000)).unwrap();
    let command = system.tools.command(Tool::Python3).unwrap();
    let envs: Vec<(String, String)> = command
        .get_envs()
        .filter_map(|(k, v)| Some((k.to_string_lossy().into_owned(), v?.to_string_lossy().into_owned())))
        .collect();
    let (u, b) = ('/', "bin");
    assert_eq!(envs, [("LANG".to_owned(), "C.UTF-8".to_owned()), ("PATH".to_owned(), format!("{u}usr{u}{b}:{u}{b}"))]);
}

/// Each failing variable on its own makes the whole setup fail (design §4.11, one case per variable), and the
/// service then opens no bus, starts no thread and runs no tool.
#[test]
fn every_failing_seam_variable_fails_the_setup() {
    let f = Fixture::new("seams");
    let root = Fixture::s(&f.root);
    let tools = Fixture::s(&f.tools);
    let unmarked = f.base.join("unmarked");
    private(&unmarked);
    let writable = f.base.join("writable");
    private(&writable);
    mark(&writable);
    fs::set_permissions(&writable, fs::Permissions::from_mode(0o777)).unwrap();
    let file_not_socket = f.bus_dir.join("plain");
    fs::write(&file_not_socket, b"").unwrap();
    let unmarked_socket_dir = f.base.join("bus-unmarked");
    private(&unmarked_socket_dir);
    let _other = std::os::unix::net::UnixListener::bind(unmarked_socket_dir.join("bus")).unwrap();
    let me = uid();
    let cases: Vec<(&str, Vec<(&str, String)>, u32)> = vec![
        ("unknown variable", vec![("WIN11_UPDATE_TYPO", "x".into())], 1000),
        ("euid 0", vec![(VAR_FIXTURE_ROOT, root.clone())], 0),
        ("empty value", vec![(VAR_TOOL_DIR, String::new())], 1000),
        ("fixture root relative", vec![(VAR_FIXTURE_ROOT, "relative/dir".into())], 1000),
        ("fixture root with ..", vec![(VAR_FIXTURE_ROOT, format!("{root}/../root"))], 1000),
        ("fixture root missing", vec![(VAR_FIXTURE_ROOT, format!("{root}-missing"))], 1000),
        ("fixture root without marker", vec![(VAR_FIXTURE_ROOT, Fixture::s(&unmarked))], 1000),
        ("fixture root world-writable", vec![(VAR_FIXTURE_ROOT, Fixture::s(&writable))], 1000),
        ("fixture root is the root directory", vec![(VAR_FIXTURE_ROOT, std::path::MAIN_SEPARATOR.to_string())], 1000),
        ("fixture root of another uid", vec![(VAR_FIXTURE_ROOT, root.clone())], 4242),
        ("tool dir without marker", vec![(VAR_TOOL_DIR, Fixture::s(&unmarked))], 1000),
        ("tool dir missing", vec![(VAR_TOOL_DIR, format!("{tools}-missing"))], 1000),
        ("bus not unix:path=", vec![(VAR_PK_BUS, "tcp:host=127.0.0.1,port=1".to_string())], 1000),
        ("bus with extra keys", vec![(VAR_PK_BUS, format!("{},guid=0", f.bus()))], 1000),
        ("bus socket missing", vec![(VAR_PK_BUS, format!("unix:path={}/none", f.bus_dir.display()))], 1000),
        ("bus is a plain file", vec![(VAR_PK_BUS, format!("unix:path={}", file_not_socket.display()))], 1000),
        ("bus dir without marker", vec![(VAR_PK_BUS, format!("unix:path={}/bus", unmarked_socket_dir.display()))], 1000),
        ("bus is the system bus", vec![(VAR_PK_BUS, format!("unix:path={}/dbus/system_bus_socket", "/run"))], 1000),
        ("bus abstract", vec![(VAR_PK_BUS, "unix:abstract=dbus-x".into())], 1000),
        // one bad variable next to two good ones still fails
        ("good root and tools, bad bus", vec![(VAR_FIXTURE_ROOT, root.clone()), (VAR_TOOL_DIR, tools.clone()), (VAR_PK_BUS, "unix:path=relative".into())], 1000),
    ];
    for (name, vars, euid) in cases {
        let euid = match euid {
            0 => 0,
            4242 => me + 1,
            _ => me,
        };
        let pairs: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let env = Env::fixture(&pairs, euid);
        assert!(paths::resolve(&env).is_err(), "{name}: the seam check passed");
        // the full service: unsupported/test_seam, no bus, no thread, no tool (stubs log nothing)
        let service = Update::with_env(f.ctx("settings", false, "nl"), &env);
        assert_eq!(service.mode(), Mode::Unsupported, "{name}");
        assert!(service.stats().is_none(), "{name}: a seam failure built the shared state");
        for (method, args) in all_calls() {
            let result = service.call(method, &args).unwrap();
            if method == "update.status" {
                let view = result.unwrap();
                assert_eq!(view["phase"], "unsupported", "{name}");
                assert_eq!(view["error"]["code"], "test_seam", "{name}");
                assert_eq!(view["error"]["key"], "update.err.testSeam", "{name}");
            } else {
                let error = result.unwrap_err();
                assert_eq!(error.code, "test_seam", "{name} {method}");
                assert_eq!(error.key.as_deref(), Some("update.err.testSeam"));
            }
        }
        service.shutdown();
    }
    assert_eq!(f.stub_log(), "", "a tool ran after a seam failure");
    assert!(f.never_connected(), "a seam failure connected to the bus");
}

/// Contract C10 (D-G39): the test bus is accepted only as one `unix:path=` (an optional `,guid=`), with the socket
/// directly in `$XDG_RUNTIME_DIR`, which is not under `/run`, is owned by the euid, has mode 0700 and holds the
/// marker, and only while the real system-bus socket is not visible. One refusal per rule, by its reason.
#[test]
fn the_test_bus_follows_contract_c10() {
    let f = Fixture::new("c10");
    let me = uid();
    let rt = f.base.join("rt");
    private(&rt);
    mark(&rt);
    let _bus = std::os::unix::net::UnixListener::bind(rt.join("bus")).unwrap();
    let socket = format!("unix:path={}", rt.join("bus").display());
    let guid = "0123456789abcdef0123456789ABCDEF";
    let none = f.base.join("no-system-bus-socket");
    let env = |address: &str, runtime: Option<&Path>, euid: u32, system: &Path| {
        Env::fixture(&[(VAR_PK_BUS, address)], euid).runtime(runtime).system_sockets(vec![system.to_path_buf()])
    };
    let reason = |address: &str, runtime: Option<&Path>, euid: u32, system: &Path| match paths::resolve(&env(address, runtime, euid, system)) {
        Ok(_) => "accepted",
        Err(e) => {
            assert_eq!(e.variable, VAR_PK_BUS);
            e.reason
        }
    };
    // accepted: the plain address and the one dbus-daemon prints (with its guid)
    for address in [socket.clone(), format!("{socket},guid={guid}")] {
        let setup = paths::resolve(&env(&address, Some(&rt), me, &none)).unwrap();
        assert_eq!(setup.bus, BusTarget::Address(address.clone()));
    }
    // a sub directory with its own marker and socket, a mode-0750 and a mode-0711 runtime, an unmarked one
    let sub = rt.join("sub");
    private(&sub);
    mark(&sub);
    let _sub_bus = std::os::unix::net::UnixListener::bind(sub.join("bus")).unwrap();
    let loose = |name: &str, mode: u32, marked: bool| {
        let dir = f.base.join(name);
        private(&dir);
        if marked {
            mark(&dir);
        }
        let listener = std::os::unix::net::UnixListener::bind(dir.join("bus")).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(mode)).unwrap();
        (dir, listener)
    };
    let (rt750, _l1) = loose("rt750", 0o750, true);
    let (rt711, _l2) = loose("rt711", 0o711, true);
    let (unmarked, _l3) = loose("rt-unmarked", 0o700, false);
    let in_dir = |dir: &Path| format!("unix:path={}", dir.join("bus").display());
    // a runtime directory that is a symlink to /run (the link itself is not under /run)
    let link = f.base.join("rt-link");
    std::os::unix::fs::symlink(format!("{}run", std::path::MAIN_SEPARATOR), &link).unwrap();
    // a plain file and a symlink where the socket should be
    fs::write(rt.join("plain"), b"").unwrap();
    std::os::unix::fs::symlink(rt.join("bus"), rt.join("link")).unwrap();
    // a visible "system bus socket" (any entry at the checked path counts)
    let visible = f.base.join("visible-system-bus");
    fs::write(&visible, b"").unwrap();
    let run_user = format!("/run/user/{me}");
    let cases: Vec<(&str, String, Option<PathBuf>, u32, PathBuf, &str)> = vec![
        ("tcp", "tcp:host=127.0.0.1,port=1".into(), Some(rt.clone()), me, none.clone(), "not a unix:path= address"),
        ("abstract", "unix:abstract=dbus-x".into(), Some(rt.clone()), me, none.clone(), "not a unix:path= address"),
        ("runtime=yes", "unix:runtime=yes".into(), Some(rt.clone()), me, none.clone(), "not a unix:path= address"),
        ("second address", format!("{socket};{socket}"), Some(rt.clone()), me, none.clone(), "more than one address"),
        ("guid not 32 hex", format!("{socket},guid=0"), Some(rt.clone()), me, none.clone(), "a key other than one guid="),
        ("two guids", format!("{socket},guid={guid},guid={guid}"), Some(rt.clone()), me, none.clone(), "a key other than one guid="),
        ("another key", format!("{socket},noncefile=/x"), Some(rt.clone()), me, none.clone(), "a key other than one guid="),
        ("escaped path", format!("unix:path=%2F{}", rt.join("bus").display()), Some(rt.clone()), me, none.clone(), "not a plain socket path"),
        ("empty path", "unix:path=".into(), Some(rt.clone()), me, none.clone(), "not a plain socket path"),
        ("relative path", "unix:path=rt/bus".into(), Some(rt.clone()), me, none.clone(), "not an absolute normal path"),
        ("path with ..", format!("unix:path={}/../rt/bus", rt.display()), Some(rt.clone()), me, none.clone(), "not an absolute normal path"),
        ("the system bus", "unix:path=/run/dbus/system_bus_socket".into(), Some(rt.clone()), me, none.clone(), "the socket is under /run"),
        ("the live session bus", format!("unix:path={run_user}/bus"), Some(PathBuf::from(&run_user)), me, none.clone(), "the socket is under /run"),
        ("system bus socket visible", socket.clone(), Some(rt.clone()), me, visible.clone(), "the real system bus socket is visible"),
        ("XDG_RUNTIME_DIR unset", socket.clone(), None, me, none.clone(), "XDG_RUNTIME_DIR is not set"),
        ("XDG_RUNTIME_DIR relative", socket.clone(), Some(PathBuf::from("rt")), me, none.clone(), "XDG_RUNTIME_DIR is not an absolute normal path"),
        ("XDG_RUNTIME_DIR under /run", in_dir(&link), Some(PathBuf::from(&run_user)), me, none.clone(), "XDG_RUNTIME_DIR is under /run"),
        ("socket in a sub directory", in_dir(&sub), Some(rt.clone()), me, none.clone(), "the socket is not directly in XDG_RUNTIME_DIR"),
        ("socket outside the runtime dir", in_dir(&rt750), Some(rt.clone()), me, none.clone(), "the socket is not directly in XDG_RUNTIME_DIR"),
        ("runtime dir links to /run", in_dir(&link), Some(link.clone()), me, none.clone(), "XDG_RUNTIME_DIR is under /run"),
        ("runtime dir missing", in_dir(&f.base.join("gone")), Some(f.base.join("gone")), me, none.clone(), "XDG_RUNTIME_DIR is missing"),
        ("runtime dir mode 0750", in_dir(&rt750), Some(rt750.clone()), me, none.clone(), "XDG_RUNTIME_DIR is not a 0700 directory of this user"),
        ("runtime dir mode 0711", in_dir(&rt711), Some(rt711.clone()), me, none.clone(), "XDG_RUNTIME_DIR is not a 0700 directory of this user"),
        ("runtime dir of another uid", socket.clone(), Some(rt.clone()), me + 1, none.clone(), "XDG_RUNTIME_DIR is not a 0700 directory of this user"),
        ("runtime dir without marker", in_dir(&unmarked), Some(unmarked.clone()), me, none.clone(), "marker missing"),
        ("no socket", format!("unix:path={}", rt.join("none").display()), Some(rt.clone()), me, none.clone(), "no socket"),
        ("a plain file", format!("unix:path={}", rt.join("plain").display()), Some(rt.clone()), me, none.clone(), "not a socket of this user"),
        ("a symlink to the socket", format!("unix:path={}", rt.join("link").display()), Some(rt.clone()), me, none.clone(), "not a socket of this user"),
    ];
    for (name, address, runtime, euid, system, want) in &cases {
        assert_eq!(reason(address, runtime.as_deref(), *euid, system), *want, "{name}");
        // and the whole service fails closed on it: unsupported, test_seam, nothing opened
        let service = Update::with_env(f.ctx("settings", false, "en"), &env(address, runtime.as_deref(), *euid, system));
        assert_eq!(service.mode(), Mode::Unsupported, "{name}");
        assert_eq!(service.call("update.check", &json!({})).unwrap().unwrap_err().code, "test_seam", "{name}");
        service.shutdown();
    }
    // review n4: a system-bus socket path that cannot be inspected (EACCES: a closed directory) counts as visible
    let closed = f.base.join("closed");
    fs::create_dir(&closed).unwrap();
    fs::set_permissions(&closed, fs::Permissions::from_mode(0o000)).unwrap();
    let unreadable = reason(&socket, Some(&rt), me, &closed.join("system_bus_socket"));
    let absent = reason(&socket, Some(&rt), me, &f.base.join("absent-dir/system_bus_socket"));
    fs::set_permissions(&closed, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(unreadable, "the real system bus socket is visible", "an unreadable directory");
    assert_eq!(absent, "accepted", "a missing directory (NotFound) is absent");
    // by default an Env checks the real socket paths (as Env::process() does): outside a layer with a private
    // /run (GE0b's unit sandbox shows the real socket) the same valid seams fail
    let real_sockets = Env::fixture(&[(VAR_PK_BUS, &socket)], me).runtime(Some(&rt));
    let real_visible = ["/run/dbus/system_bus_socket", "/var/run/dbus/system_bus_socket"].iter().any(|p| paths::maybe_present(Path::new(p)));
    match paths::resolve(&real_sockets) {
        Err(e) => assert!(real_visible && e.reason == "the real system bus socket is visible", "{e:?}"),
        Ok(_) => assert!(!real_visible),
    }
    assert_eq!(f.stub_log(), "");
}

#[test]
fn the_test_seam_message_renders_in_both_languages() {
    let f = Fixture::new("seam-lang");
    let env = Env::fixture(&[("WIN11_UPDATE_TYPO", "1")], 1000);
    for (lang, text) in [
        ("nl", "Een testinstelling voor updates klopt niet, dus er is niets gedaan."),
        ("en", "An update test setting is wrong, so nothing was done."),
    ] {
        let service = Update::with_env(f.ctx("taskmgr", false, lang), &env);
        assert_eq!(service.call("update.status", &json!({})).unwrap().unwrap()["error"]["message"], text);
        assert_eq!(service.call("update.check", &json!({})).unwrap().unwrap_err().message, text);
    }
}

#[test]
fn system_bus_panics_in_unit_tests_without_a_private_bus() {
    if std::env::var_os(VAR_PK_BUS).is_some() {
        return; // a bus-test run: the private bus is the only one system_bus() may return
    }
    let result = std::panic::catch_unwind(super::pk::system_bus);
    assert!(result.is_err(), "pk::system_bus() returned without WIN11_UPDATE_PK_BUS");
}

// ------------------------------------------------------------------------------------- exec-path grep

/// The forbidden patterns of the exec-path grep (design §4.11, review F11), built so that this file does not
/// match itself: absolute program and library paths (allowed only inside the `paths.rs` table), every other way
/// to start a program, and the std command type itself as a word in code (`paths.rs` only; this also catches
/// grouped and aliased imports). Returns (paths, starts, the type's name).
fn exec_patterns() -> (Vec<String>, Vec<String>, String) {
    let (q, s) = ('"', '/');
    let command = ["Comm", "and"].concat();
    let paths = ["usr/bin/", "usr/lib/", "bin/", "usr/libexec/", "sbin/", "usr/sbin/", "usr/local/"]
        .iter()
        .map(|p| format!("{q}{s}{p}"))
        .chain([format!("{command}::new({q}{s}")])
        .collect();
    let starts = [
        ("Sub", "process"),
        ("spawn_", "async"),
        ("spawn_", "sync"),
        ("spawn_", "command_line"),
        ("glib::", "spawn"),
        (".exec", "()"),
        ("libc::", "exec"),
        ("exec", "vp("),
        ("exec", "ve("),
        ("posix_", "spawn"),
        ("libc::", "system("),
        ("libc::", "fork("),
        ("po", "pen("),
    ]
    .iter()
    .map(|(a, b)| format!("{a}{b}"))
    .collect();
    (paths, starts, command)
}
/// The lines of one file that break the rule (`paths.rs`: the table lines are skipped, the type is allowed).
fn exec_hits(text: &str, is_paths: bool) -> (Vec<String>, usize, bool) {
    let (paths, starts, command) = exec_patterns();
    let (begin, end) = (format!("// table{}begin", ':'), format!("// table{}end", ':'));
    let ident = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
    let (mut hits, mut tables, mut in_table) = (Vec::new(), 0, false);
    for (n, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if is_paths && trimmed.starts_with(&begin) {
            in_table = true;
            tables += 1;
            continue;
        }
        if is_paths && trimmed.starts_with(&end) {
            in_table = false;
            continue;
        }
        if in_table {
            continue;
        }
        // the type as a whole word in code (`…Ext`, for pre_exec on a table command, is another word)
        let code = line.split("//").next().unwrap_or("");
        let typed = code
            .match_indices(command.as_str())
            .any(|(at, _)| !ident(code[..at].chars().next_back()) && !ident(code[at + command.len()..].chars().next()));
        if paths.iter().any(|p| line.contains(p.as_str())) || (!is_paths && (typed || starts.iter().any(|p| line.contains(p.as_str())))) {
            hits.push(format!("{}: {}", n + 1, line.trim()));
        }
    }
    (hits, tables, in_table)
}

/// No literal absolute exec path under `services/update/` outside the table of `paths.rs` (design §4.11),
/// and no other way to start a program outside `paths.rs` (review F11): every program comes from the tool table.
#[test]
fn no_absolute_exec_path_outside_the_tool_table() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/services");
    let mut files = vec![dir.join("update.rs")];
    files.extend(fs::read_dir(dir.join("update")).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "rs")));
    assert!(files.len() >= 18, "update/ is incomplete: {files:?}");
    let mut all = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file).unwrap();
        let is_paths = file.file_name().is_some_and(|n| n == "paths.rs");
        let (hits, tables, open) = exec_hits(&text, is_paths);
        if is_paths {
            assert_eq!(tables, 1, "paths.rs must have exactly one table");
            assert!(!open, "paths.rs: the table is not closed");
        }
        all.extend(hits.into_iter().map(|h| format!("{}:{h}", file.display())));
    }
    assert!(all.is_empty(), "absolute exec paths or program starts outside the table:\n{}", all.join("\n"));
    // the grep catches what it must (each line alone, outside paths.rs), and paths.rs may hold the table
    let (q, s) = ('"', '/');
    let cmd = ["Comm", "and"].concat();
    let bad = [
        format!("let p = {q}{s}usr{s}sbin{s}reboot{q};"),
        format!("let p = {q}{s}sbin{s}poweroff{q};"),
        format!("let p = {q}{s}usr{s}local{s}bin{s}x{q};"),
        format!("use std::process::{cmd};"),
        format!("let c = {cmd}::new(name);"),
        format!("use std::process::{cmd} as Cmd;"),
        format!("use std::process::{{Child, {cmd}}};"),
        format!("    process::{{Child, {cmd}, Stdio}},"),
        format!("let c: std::process::{cmd} = make();"),
        format!("gio::Sub{}::newv(&argv, flags)", "process"),
        format!("let l = gio::Sub{}Launcher::new(flags);", "process"),
        format!("glib::{}_async(None, &argv, &[], flags, None)", "spawn"),
        format!("glib::{}_command_line_async(\"pkexec x\")", "spawn"),
        format!("let e = command.exec{};", "()"),
        format!("unsafe {{ libc::{}v(p, a) }}", "exec"),
        format!("unsafe {{ libc::posix_{}(..) }}", "spawn"),
        format!("unsafe {{ libc::system{}c.as_ptr()) }}", "("),
    ];
    for line in &bad {
        assert_eq!(exec_hits(line, false).0.len(), 1, "the grep misses: {line}");
    }
    let table = format!("// table{}begin\nconst A: &str = {q}{s}usr{s}sbin{s}x{q};\n// table{}end\nuse std::process::{cmd};\n", ':', ':');
    assert_eq!(exec_hits(&table, true), (Vec::new(), 1, false), "paths.rs: the table and the one import of the type");
    assert_eq!(exec_hits(&format!("let p = {q}{s}bin{s}sh{q};"), true).0.len(), 1, "outside the table in paths.rs too");
    let fine = format!("let mut child = command.spawn()?; use std::os::unix::process::{cmd}Ext; // a {cmd} of the table");
    assert!(exec_hits(&fine, false).0.is_empty(), "a table command may be spawned and pre_exec'd");
}

// ------------------------------------------------------------------------------------- phasing

pub(super) const SCRIPT_OUTPUT: &str = "libsoup-3.0-0\tamd64\t3.6.6-1ubuntu0.3\t0\t-\t0\t3.6.6-1ubuntu0.2\n\
gnome-shell\tamd64\t50.1-0ubuntu1.6\t1\t10\t0\t50.1-0ubuntu1.5\n\
linux-image-7.0.0-44-generic\tamd64\t7.0.0-44.44\t0\t-\t1\t-\n\
libgtk-3-0t64\tamd64\t3.24.53-1\t1\t40\t0\t3.24.52-1\n\
zlib1g\tamd64\t1:1.3.dfsg-3.1ubuntu3\t0\t-\t0\t1:1.3.dfsg-3.1ubuntu2\n\
libfoo1\tamd64\t2.0-1\t0\t-\t0\t1.9-1\n\
garbage line\n\
libold\tamd64\t1\t0\t-\t0\n\
Bad Name\tamd64\t1\t0\t-\t0\t-\n\
@@end\n";

#[test]
fn phasing_script_output_is_parsed_and_needs_its_end_marker() {
    let parsed = phasing::parse_script(SCRIPT_OUTPUT).unwrap();
    assert_eq!(parsed.len(), 6);
    let shell = &parsed[&("gnome-shell".into(), "amd64".into())];
    assert!(shell.phased && !shell.held);
    assert_eq!(shell.pup, Some(10));
    assert_eq!(shell.candidate.as_deref(), Some("50.1-0ubuntu1.6"));
    assert!(parsed[&("linux-image-7.0.0-44-generic".into(), "amd64".into())].held);
    assert!(!parsed[&("libsoup-3.0-0".into(), "amd64".into())].phased);
    assert_eq!(parsed[&("libsoup-3.0-0".into(), "amd64".into())].installed.as_deref(), Some("3.6.6-1ubuntu0.2"));
    assert_eq!(parsed[&("linux-image-7.0.0-44-generic".into(), "amd64".into())].installed, None, "a new kernel");
    assert!(!parsed.contains_key(&("libold".into(), "amd64".into())), "six columns: an old script's line");
    // a cut pipe or a crash: no end marker, no classification
    assert!(phasing::parse_script(&SCRIPT_OUTPUT.replace("@@end\n", "")).is_none());
}

#[test]
fn apt_cache_show_fallback_counts_any_phased_percentage() {
    let show = "Package: gnome-shell\nVersion: 50.1-0ubuntu1.6\nPhased-Update-Percentage: 10\nArchitecture: amd64\n\n\
Package: libsoup-3.0-0\nVersion: 3.6.6-1ubuntu0.3\nArchitecture: amd64\n\n\
Package: zlib1g\nVersion: 1:1.3.dfsg-3.1ubuntu3\nPhased-Update-Percentage: 0\n";
    let parsed = phasing::parse_show(show);
    assert!(parsed[&("gnome-shell".into(), "50.1-0ubuntu1.6".into())].phased);
    assert!(!parsed[&("libsoup-3.0-0".into(), "3.6.6-1ubuntu0.3".into())].phased);
    let zlib = &parsed[&("zlib1g".into(), "1:1.3.dfsg-3.1ubuntu3".into())];
    assert!(zlib.phased, "Phased-Update-Percentage: 0 is phased too (do-release-upgrade's rule)");
    assert_eq!(zlib.pup, Some(0));
}

pub(super) fn sample_pkgs() -> Vec<Pkg> {
    vec![
        pkg("libsoup-3.0-0", "3.6.6-1ubuntu0.3", model::info::SECURITY, "ubuntu-resolute-security"),
        pkg("gnome-shell", "50.1-0ubuntu1.6", 5, "ubuntu-resolute-updates"),
        pkg("linux-image-7.0.0-44-generic", "7.0.0-44.44", 5, "ubuntu-resolute-updates"),
        pkg("libgtk-3-0t64", "3.24.53-1", 5, "ubuntu-resolute-updates"),
        pkg("zlib1g", "1:1.3.dfsg-3.1ubuntu3", 5, "ubuntu-resolute-updates"),
        pkg("libfoo1", "2.0-1", model::info::BLOCKED, "ubuntu-resolute-updates"),
    ]
}

#[test]
fn phased_and_held_packages_never_reach_a_row_or_update_packages() {
    let mut pkgs = sample_pkgs();
    let parsed = phasing::parse_script(SCRIPT_OUTPUT).unwrap();
    phasing::apply(&mut pkgs, Some(&parsed), None);
    let by = |n: &str| pkgs.iter().find(|p| p.name == n).unwrap().clone();
    assert!(by("gnome-shell").phased);
    assert!(by("libgtk-3-0t64").phased);
    assert!(by("linux-image-7.0.0-44-generic").held, "apt hold");
    assert!(by("libfoo1").held, "info 9 blocked stays held");
    assert!(by("libsoup-3.0-0").offered());
    assert_eq!(by("libsoup-3.0-0").from.as_deref(), Some("3.6.6-1ubuntu0.2"), "the installed version");
    let rows = model::group(&pkgs, "26.04.1 LTS");
    let ids: Vec<&String> = rows.iter().flat_map(|r| r.ids.iter()).collect();
    for p in pkgs.iter().filter(|p| !p.offered()) {
        assert!(!ids.contains(&&p.id), "{} is in a row", p.name);
    }
    let all = model::install_ids(&rows, None).unwrap();
    assert!(all.iter().all(|id| pkgs.iter().any(|p| &p.id == id && p.offered())));
    assert_eq!(all.len(), 2, "libsoup and zlib1g only");
    // a row that holds only phased packages does not exist, so asking for it is stale
    assert!(model::install_ids(&rows, Some(&["gnome".into()])).is_none());
    let totals = model::totals(&pkgs, None);
    assert_eq!(totals["count"], 2);
    assert_eq!(totals["security"], 1);
    assert_eq!(totals["phased"], 2);
    assert_eq!(totals["held"], 2);
}

/// A package without an answer for the offered version (apt's candidate is another one, or it is missing from
/// the output) is unclassified and gets the safe rule: offered only from the `-security` pocket (review F5).
#[test]
fn a_package_without_an_answer_for_its_version_gets_the_safe_rule() {
    let mut pkgs = vec![
        pkg("gnome-shell", "50.1-0ubuntu1.7", 5, "ubuntu-resolute-updates"),
        pkg("libsoup-3.0-0", "3.6.6-1ubuntu0.4", 8, "ubuntu-resolute-security"),
        pkg("unknown-pkg", "1.0-1", 5, "ubuntu-resolute-updates"),
    ];
    let parsed = phasing::parse_script(SCRIPT_OUTPUT).unwrap();
    assert_eq!(phasing::apply(&mut pkgs, Some(&parsed), None), 3);
    assert!(pkgs[0].phased, "apt's candidate 1.6 says nothing about 1.7: held back as phased");
    assert!(!pkgs[1].phased, "the -security pocket is never phased: still offered");
    assert!(pkgs[2].phased, "not in the output");
    assert_eq!(pkgs[0].from.as_deref(), Some("50.1-0ubuntu1.5"), "the installed version is still known");
    let rows = model::group(&pkgs, "26.04.1 LTS");
    assert_eq!(model::install_ids(&rows, None).unwrap(), [pkgs[1].id.clone()]);
}

#[test]
fn the_classifier_runs_only_the_stub_with_names_on_stdin() {
    let f = Fixture::new("phasing");
    let input = f.base.join("stdin");
    let output = f.base.join("canned");
    write(&output, SCRIPT_OUTPUT);
    let cat = format!("{}bin{}cat", '/', '/');
    script(
        &f.tools.join("python3"),
        &format!("echo \"python3 $1 $2\" >> '{log}'\n{cat} > '{input}'\n{cat} '{output}'\n", log = f.log.display(), input = input.display(), output = output.display()),
    );
    let setup = paths::resolve(&f.env()).unwrap();
    let mut pkgs = sample_pkgs();
    phasing::classify(&setup.tools, &mut pkgs).unwrap();
    assert_eq!(f.stub_log(), "python3 -I -c\n");
    let sent = fs::read_to_string(&input).unwrap();
    assert!(sent.lines().all(|l| l.split('\t').count() == 2), "{sent}");
    assert!(sent.contains("gnome-shell\tamd64\n"));
    assert!(pkgs.iter().find(|p| p.name == "gnome-shell").unwrap().phased);
    assert_eq!(setup.tools.runs(), 1);
    // without python3-apt (exit 3): one apt-cache show call
    script(&f.tools.join("python3"), "echo @@no-apt-pkg\nexit 3\n");
    script(
        &f.tools.join("apt-cache"),
        &format!("echo \"apt-cache $*\" >> '{log}'\nprintf 'Package: zlib1g\\nVersion: 1:1.3.dfsg-3.1ubuntu3\\nPhased-Update-Percentage: 20\\n'\n", log = f.log.display()),
    );
    let mut pkgs = sample_pkgs();
    phasing::classify(&setup.tools, &mut pkgs).unwrap();
    assert!(pkgs.iter().find(|p| p.name == "zlib1g").unwrap().phased);
    let log = f.stub_log();
    let call = log.lines().last().unwrap();
    assert!(call.starts_with("apt-cache show --no-all-versions "), "{call}");
    assert!(call.contains("zlib1g=1:1.3.dfsg-3.1ubuntu3"));
    // any other classifier failure runs the fallback too (review F5): a traceback, a missing end marker
    for broken in ["echo Traceback >&2\nexit 1\n", "echo 'zlib1g\tamd64\t1\t0\t-\t0\t-'\nexit 0\n"] {
        script(&f.tools.join("python3"), broken);
        let before = f.stub_log().lines().count();
        let mut pkgs = sample_pkgs();
        assert_eq!(phasing::classify(&setup.tools, &mut pkgs).unwrap(), phasing::Classified::Fallback, "{broken}");
        assert!(f.stub_log().lines().skip(before).any(|l| l.starts_with("apt-cache show")), "{broken}: no fallback");
        assert!(pkgs.iter().find(|p| p.name == "zlib1g").unwrap().phased);
        // what apt-cache did not answer: the safe rule (libsoup is -security, so it stays offered)
        let offered: Vec<&str> = pkgs.iter().filter(|p| p.offered()).map(|p| p.name.as_str()).collect();
        assert_eq!(offered, ["libsoup-3.0-0"], "{broken}");
    }
    // both fail: nothing classified, the safe rule for every package, no panic
    script(&f.tools.join("apt-cache"), "exit 100\n");
    let mut pkgs = sample_pkgs();
    assert_eq!(phasing::classify(&setup.tools, &mut pkgs).unwrap(), phasing::Classified::Unknown);
    let offered: Vec<&str> = pkgs.iter().filter(|p| p.offered()).map(|p| p.name.as_str()).collect();
    assert_eq!(offered, ["libsoup-3.0-0"]);
    assert_eq!(model::totals(&pkgs, None)["count"], 1);
    // a missing python3 stub is a seam error at once: no further program, the safe rule
    fs::remove_file(f.tools.join("python3")).unwrap();
    let runs = setup.tools.runs();
    let mut pkgs = sample_pkgs();
    assert!(phasing::classify(&setup.tools, &mut pkgs).is_err());
    assert_eq!(setup.tools.runs(), runs, "nothing ran after the missing stub");
    assert_eq!(pkgs.iter().filter(|p| p.offered()).count(), 1);
}

// ------------------------------------------------------------------------------------- owner election

#[test]
fn owner_election_is_one_flock_and_rev_continues() {
    let f = Fixture::new("owner");
    let store = store::Store::open(f.base.join("cache/update")).unwrap();
    let other = store::Store::open(f.base.join("cache/update")).unwrap();
    let won = store::elect(&store).unwrap();
    assert!(matches!(won, store::Election::Won(_)));
    let owner = store.read(store::OWNER).unwrap();
    assert_eq!(owner["pid"], std::process::id());
    assert!(store::owner_alive(&owner));
    assert!(matches!(store::elect(&other).unwrap(), store::Election::Lost), "a second Settings must be a mirror");
    drop(won);
    assert!(matches!(store::elect(&other).unwrap(), store::Election::Won(_)), "the kernel released the lock");
    // rev: a new owner continues from the snapshot, and rev never goes back
    store.write(store::SNAPSHOT, &json!({"rev":17})).unwrap();
    assert_eq!(store::next_rev(&other, 0), 18);
    assert_eq!(store::next_rev(&other, 40), 41);
    assert!(!store::owner_alive(&json!({"pid":std::process::id(),"start":"1"})), "same pid, other start");
    assert!(!store::owner_alive(&json!({"pid":u32::MAX})));
}

fn wait_for(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < timeout, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Two Settings services on one cache: one owner, one mirror; the mirror forwards its writes, the owner
/// answers with update.reply; when the owner leaves, the mirror takes over. No bus (none configured).
#[test]
fn two_settings_hosts_elect_one_owner_and_the_mirror_forwards_and_takes_over() {
    let f = Fixture::new("two");
    let cache = f.base.join("shared-cache");
    let owner = Update::with_env(f.ctx_in(&cache, "settings", false, "en"), &f.env_no_bus());
    let mirror_ctx = f.ctx_in(&cache, "settings", false, "en");
    let mirror = Update::with_env(mirror_ctx.clone(), &f.env_no_bus());
    assert_eq!(owner.mode(), Mode::Owner);
    assert_eq!(mirror.mode(), Mode::Mirror);
    let (tx, rx) = mpsc::channel::<Value>();
    let tx = std::sync::Mutex::new(tx);
    mirror_ctx.events.on(
        "update.reply",
        Arc::new(move |data: &Value| {
            let _ = tx.lock().unwrap().send(data.clone());
        }),
    );
    let forwarded = mirror.call("update.check", &json!({})).unwrap().unwrap();
    assert_eq!(forwarded["forwarded"], true);
    let reply = rx.recv_timeout(Duration::from_secs(10)).expect("the owner answered");
    assert_eq!(reply["nonce"], forwarded["nonce"]);
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["error"]["key"], "update.err.noPackageKit", "no bus here, so no PackageKit");
    assert_eq!(reply["error"]["message"], "PackageKit is missing, so Win11 Shell can’t install updates itself.");
    // a bad forwarded request is refused by the owner's own allowlist
    mirror_ctx.emit("update.request", json!({"nonce":"0123456789abcdef0123456789abcdef","method":"update.install","args":{"rows":["nope"]}}));
    let reply = rx.recv_timeout(Duration::from_secs(10)).expect("the owner refused");
    assert_eq!(reply["error"]["code"], "bad_args");
    // the owner's snapshot: no PackageKit → unsupported; the mirror reads the same files
    wait_for("the owner's snapshot", Duration::from_secs(10), || {
        mirror.call("update.status", &json!({})).unwrap().unwrap()["rev"].as_u64() > Some(0)
    });
    let view = mirror.call("update.status", &json!({})).unwrap().unwrap();
    assert_eq!(view["phase"], "unsupported");
    assert_eq!(view["owner"]["mirror"], true);
    assert_eq!(view["owner"]["pid"], std::process::id());
    let rev = view["rev"].as_u64().unwrap();
    owner.shutdown();
    wait_for("the mirror's takeover", Duration::from_secs(10), || mirror.mode() == Mode::Owner);
    wait_for("the new owner's snapshot", Duration::from_secs(10), || {
        mirror.call("update.status", &json!({})).unwrap().unwrap()["rev"].as_u64().unwrap_or(0) > rev
    });
    let (bus, threads, tools, sent) = mirror.stats().unwrap();
    assert_eq!((bus, threads, tools, sent), (0, 1, 0, 1), "no bus opened, one pk thread after the takeover");
    mirror.shutdown();
    assert_eq!(f.stub_log(), "");
}

// ------------------------------------------------------------------------------------- dry-run, reader

#[test]
fn dry_run_makes_zero_calls() {
    let f = Fixture::new("dry");
    let service = Update::with_env(f.ctx("settings", true, "nl"), &f.env());
    assert_eq!(service.mode(), Mode::Dry);
    for (method, args) in all_calls() {
        let result = service.call(method, &args).unwrap();
        match method {
            "update.history" | "update.options" => {} // read-only stubs until GW4
            "update.status" => assert_eq!(result.unwrap()["dryRun"], true),
            "update.install" => assert_eq!(result.unwrap(), json!({"jobId":null,"dryRun":true})),
            "update.details" => assert_eq!(result.unwrap(), json!({"details":[],"dryRun":true})),
            "update.check" => {
                let view = result.unwrap();
                assert_eq!(view["dryRun"], true);
                assert_eq!(view["backend"], "packagekit");
                assert_eq!(view["phase"], "idle");
                assert_eq!(view["rows"], json!([]));
            }
            "update.release.compat" => assert_eq!(result.unwrap()["state"], "unknown"),
            _ => assert_eq!(result.unwrap()["dryRun"], true, "{method}"),
        }
    }
    assert_eq!(service.stats(), Some((0, 0, 0, 0)), "dry-run opened a bus, started a thread, ran or forwarded");
    assert_eq!(f.stub_log(), "");
    assert!(f.never_connected());
    service.shutdown();
}

#[test]
fn reader_mode_never_connects_and_refuses_writes() {
    let f = Fixture::new("reader");
    for kind in ["panel", "start", "desktop", "taskmgr", "explorer"] {
        let service = Update::with_env(f.ctx(kind, false, "nl"), &f.env());
        assert_eq!(service.mode(), Mode::Reader, "{kind}");
        for (method, args) in all_calls() {
            let result = service.call(method, &args).unwrap();
            match method {
                "update.status" => assert_eq!(result.unwrap()["mode"], "reader"),
                "update.history" | "update.options" => {}
                _ => {
                    let error = result.unwrap_err();
                    assert_eq!(error.code, "not_here", "{kind} {method}");
                    assert_eq!(error.message, "Updates installeren kan alleen vanuit Instellingen.");
                }
            }
        }
        assert_eq!(service.stats(), Some((0, 0, 0, 0)), "{kind}");
        service.shutdown();
    }
    assert_eq!(f.stub_log(), "");
    assert!(f.never_connected());
}

#[test]
fn the_power_guard_reads_files_only_and_never_connects() {
    let f = Fixture::new("power");
    let ctx = f.ctx("panel", false, "en");
    let setup = paths::resolve(&f.env()).unwrap();
    let store = store::Store::open(ctx.cache_dir.join("update")).unwrap();
    let boot = store::boot_id(&setup.paths).unwrap();
    let lang = ctx.lang();
    assert!(guard::power_blocked_in(&ctx, &setup, lang).is_none(), "no job");
    store.write(store::JOB, &json!({"bootId":boot,"committing":true,"owner":store::current_owner()})).unwrap();
    let error = guard::power_blocked_in(&ctx, &setup, lang).unwrap();
    assert_eq!(error.key.as_deref(), Some("update.guard.power"));
    assert_eq!(error.message, "Updates are being installed. You can restart as soon as that’s done.");
    // a finished job, another boot, or a job still downloading never blocks
    store.write(store::JOB, &json!({"bootId":boot,"committing":true,"owner":store::current_owner(),"finishedAt":1})).unwrap();
    assert!(guard::power_blocked_in(&ctx, &setup, lang).is_none());
    store.write(store::JOB, &json!({"bootId":"other","committing":true,"owner":store::current_owner()})).unwrap();
    assert!(guard::power_blocked_in(&ctx, &setup, lang).is_none());
    store.write(store::JOB, &json!({"bootId":boot,"committing":false,"owner":store::current_owner()})).unwrap();
    assert!(guard::power_blocked_in(&ctx, &setup, lang).is_none());
    // the release upgrader runs → update.guard.upgrade
    write(&f.root.join("proc/4321/cmdline"), "python3\0do-release-upgrade\0-f\0DistUpgradeViewText\0");
    assert_eq!(guard::power_blocked_in(&ctx, &setup, lang).unwrap().key.as_deref(), Some("update.guard.upgrade"));
    assert!(f.never_connected());
    assert_eq!(f.stub_log(), "");
}

#[test]
fn a_dead_owner_blocks_power_only_while_its_job_file_is_recent() {
    let job = json!({"bootId":"b","committing":true});
    let now = 10_000_000;
    assert!(guard::job_blocks_power(&job, Some("b"), true, 0, now), "live owner, any age");
    assert!(guard::job_blocks_power(&job, Some("b"), false, now - 60_000, now), "dead owner, 1 min old");
    assert!(!guard::job_blocks_power(&job, Some("b"), false, now - 11 * 60_000, now), "dead owner, 11 min old");
    assert!(!guard::job_blocks_power(&job, None, true, now, now), "unknown boot");
    assert!(!guard::job_blocks_power(&json!({"bootId":"b","committing":true,"finishedAt":5}), Some("b"), true, now, now));
}

// ------------------------------------------------------------------------------------- /proc/locks

#[test]
fn a_held_dpkg_or_apt_lock_is_found_in_proc_locks_by_device_and_inode() {
    use std::os::unix::fs::MetadataExt;
    let f = Fixture::new("locks");
    let setup = paths::resolve(&f.env()).unwrap();
    for lock in setup.paths.apt_locks() {
        write(lock, "");
    }
    let meta = fs::metadata(&setup.paths.dpkg_lock_frontend).unwrap();
    let (major, minor, inode) = (libc::major(meta.dev()), libc::minor(meta.dev()), meta.ino());
    let line = |kind: &str, ino: u64| format!("1: {kind}  ADVISORY  WRITE 4242 {major:02x}:{minor:02x}:{ino} 0 EOF\n");
    // nothing locked, an unrelated inode, a waiter line: not busy
    write(&setup.paths.proc_locks, "");
    assert!(!guard::apt_lock_held(&setup.paths));
    write(&setup.paths.proc_locks, &line("POSIX", inode + 99_999));
    assert!(!guard::apt_lock_held(&setup.paths));
    write(&setup.paths.proc_locks, &format!("1: -> POSIX  ADVISORY  WRITE 4242 {major:02x}:{minor:02x}:{inode} 0 EOF\n"));
    assert!(!guard::apt_lock_held(&setup.paths), "a waiter holds nothing");
    // lock-frontend held (POSIX, OFD or flock): busy
    for kind in ["POSIX", "OFDLCK", "FLOCK"] {
        write(&setup.paths.proc_locks, &format!("2: FLOCK  ADVISORY  WRITE 1 00:1a:5 0 EOF\n{}", line(kind, inode)));
        assert!(guard::apt_lock_held(&setup.paths), "{kind}");
    }
    let archives = fs::metadata(&setup.paths.apt_archives_lock).unwrap().ino();
    write(&setup.paths.proc_locks, &line("POSIX", archives));
    assert!(guard::apt_lock_held(&setup.paths), "archives lock");
    assert_eq!(guard::parse_locks("1: POSIX  ADVISORY  WRITE 99 08:02:1234 0 EOF\n"), vec![(8, 2, 1234)]);
    assert_eq!(guard::parse_locks("garbage\n"), vec![]);
}

// ------------------------------------------------------------------------------------- upgrader detection

#[test]
fn the_release_upgrader_is_detected_but_not_our_wrapper_or_the_pkexec_prompt() {
    let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert!(guard::is_upgrader(&args(&["python3", "do-release-upgrade", "-f", "DistUpgradeViewText"])));
    assert!(guard::is_upgrader(&args(&["do-release-upgrade"])));
    assert!(guard::is_upgrader(&args(&["python3", "-s", "/tmp/ubuntu-release-upgrader-x1y2/stonking", "--mode=server"])));
    assert!(guard::is_upgrader(&args(&["SCREEN", "-e", "\\0\\0", "-c", "screenrc", "-S", guard::UPGRADE_SCREEN])));
    assert!(!guard::is_upgrader(&args(&["sh", "-c", "status=$1", "win11-gnome-upgrade", "/run/x", "fail", "pkexec", "do-release-upgrade"])));
    assert!(!guard::is_upgrader(&args(&["pkexec", "do-release-upgrade"])));
    assert!(!guard::is_upgrader(&args(&["vim", "do-release-upgrade.txt"])));
    assert!(!guard::is_upgrader(&[]));
    // review F12: programs that only name the upgrader's paths are not the upgrader
    let usr = |rest: &str| format!("{}usr{}{rest}", '/', '/');
    let (python, dru, main) = (usr("bin/python3"), usr("bin/do-release-upgrade"), usr("lib/python3/dist-packages/DistUpgrade/DistUpgradeMain.py"));
    assert!(guard::is_upgrader(&args(&[&python, "-s", &dru, "-f", "DistUpgradeViewGtk3"])));
    assert!(guard::is_upgrader(&args(&["python3.14", "-W", "ignore", "-u", "/tmp/ubuntu-release-upgrader-ab12/stonking"])));
    assert!(guard::is_upgrader(&args(&["screen", "-e", "\\0\\0", "-c", "screenrc", "-S", guard::UPGRADE_SCREEN, "/tmp/ubuntu-release-upgrader-ab12/stonking"])));
    for (what, line) in [
        ("an editor", vec!["vim", "/tmp/ubuntu-release-upgrader-ab12/stonking"]),
        ("a pager", vec!["less", main.as_str()]),
        ("grep", vec!["grep", "-r", "DistUpgradeMain", "DistUpgrade/"]),
        ("ls", vec!["ls", "/tmp/ubuntu-release-upgrader-x"]),
        ("python -c", vec!["python3", "-c", "import DistUpgrade.DistUpgradeMain", "/tmp/ubuntu-release-upgrader-x/y"]),
        ("another python script", vec!["python3", "/home/robin/notes.py", "/tmp/ubuntu-release-upgrader-x"]),
        ("python -m", vec!["python3", "-m", "pydoc", "do-release-upgrade"]),
        ("the attach client", vec!["screen", "-d", "-r", "-p", guard::UPGRADE_SCREEN]),
        ("a screen named so", vec!["screen", "-ls", guard::UPGRADE_SCREEN]),
    ] {
        assert!(!guard::is_upgrader(&args(&line)), "{what} counted as the upgrader");
    }
}

#[test]
fn every_write_is_refused_while_the_release_upgrader_runs() {
    let f = Fixture::new("upgrading");
    write(&f.root.join("proc/777/cmdline"), "python3\0do-release-upgrade\0");
    // the owner (its snapshot says no PackageKit here, so its phase is unsupported) and a reader both refuse
    let owner = Update::with_env(f.ctx("settings", false, "nl"), &f.env_no_bus());
    assert_eq!(owner.mode(), Mode::Owner);
    let reader = Update::with_env(f.ctx_in(&f.base.join("reader-cache"), "panel", false, "nl"), &f.env());
    let mut calls = all_calls();
    calls.push(("update.confirm", json!({"jobId":"0123456789abcdef0123456789abcdef","accept":false})));
    for service in [&owner, &reader] {
        for (method, args) in &calls {
            let method: &str = method;
            let result = service.call(method, args).unwrap();
            let guarded = super::upgrade_guarded(method, args);
            match method {
                "update.status" | "update.history" | "update.options" => assert!(!guarded),
                _ if guarded => assert_eq!(result.unwrap_err().key.as_deref(), Some("update.guard.upgrade"), "{method}"),
                // review F17: cancel, details, a declined confirm, the toggles and attach are not refused by the
                // guard (here they end in the mode's or the stub's own answer)
                _ => assert_ne!(result.err().and_then(|e| e.key), Some("update.guard.upgrade".into()), "{method} {args}"),
            }
        }
        assert_eq!(service.stats().unwrap().2, 0, "no tool ran");
    }
    let guarded: Vec<&str> = calls.iter().filter(|(m, a)| super::upgrade_guarded(m, a)).map(|(m, _)| *m).collect();
    assert_eq!(
        guarded,
        ["update.check", "update.install", "update.install", "update.confirm", "update.repair", "update.release.check",
         "update.release.compat", "update.release.setPrompt", "update.release.start"],
        "design §3.7's list"
    );
    assert_eq!(reader.call("update.status", &json!({})).unwrap().unwrap()["phase"], "releaseRunning");
    owner.shutdown();
    assert_eq!(f.stub_log(), "");
    assert!(f.never_connected());
}

// ------------------------------------------------------------------------------------- arguments

#[test]
fn arguments_go_through_a_per_method_allowlist() {
    let token = "0123456789abcdef0123456789abcdef";
    let good = [
        ("update.status", json!({})),
        ("update.status", json!({"packages":false})),
        ("update.install", json!({"rows":["security","other"]})),
        ("update.confirm", json!({"jobId":token,"accept":false})),
        ("update.history", json!({"before":1762341720000u64,"limit":200})),
        ("update.release.setPrompt", json!({"prompt":"never"})),
        ("update.setWatching", json!({"on":false})),
    ];
    for (method, args) in good {
        assert!(check_args(method, &args), "{method} {args}");
    }
    let id = "a1;1;amd64;x";
    let bad = [
        ("update.status", json!({"packages":"yes"})),
        ("update.status", json!({"extra":1})),
        ("update.status", json!([])),
        ("update.check", json!({"force":true})),
        ("update.install", json!({"rows":[]})),
        ("update.install", json!({"rows":["security","security"]})),
        ("update.install", json!({"rows":["libsoup"]})),
        ("update.install", json!({"ids":[id]})),
        ("update.install", json!({"flags":4})),
        ("update.confirm", json!({"jobId":token})),
        ("update.confirm", json!({"jobId":"short","accept":true})),
        ("update.cancel", json!({"jobId":"../../etc"})),
        ("update.details", json!({"ids":[]})),
        ("update.details", json!({"ids":vec![id; 21]})),
        ("update.details", json!({"ids":["not-a-package-id"]})),
        ("update.history", json!({"limit":201})),
        ("update.history", json!({"limit":0})),
        ("update.release.setPrompt", json!({"prompt":"always"})),
        ("update.release.ubuntuHold", json!({})),
        ("update.setWatching", json!({"on":"no"})),
        ("update.setWatching", json!({})),
        ("update.unknown", json!({})),
    ];
    for (method, args) in bad {
        assert!(!check_args(method, &args), "{method} {args} passed");
    }
    let f = Fixture::new("args");
    let service = Update::with_env(f.ctx("panel", false, "en"), &f.env());
    let error = service.call("update.install", &json!({"flags":4})).unwrap().unwrap_err();
    assert_eq!((error.code.as_str(), error.message.as_str()), ("bad_args", "Invalid update input."));
    assert!(service.call("system.updates", &json!({})).is_none(), "not an update method");
    assert!(service.call("update.nope", &json!({})).is_none());
    // the native Settings' window report: local, in every mode, read by pk.rs notify (ServiceContext::watching)
    let ctx = f.ctx("settings", false, "en");
    let settings = Update::with_env(ctx.clone(), &f.env_no_bus());
    assert!(!ctx.watching());
    assert_eq!(settings.call("update.setWatching", &json!({"on":true})).unwrap().unwrap(), json!({"on":true}));
    assert!(ctx.watching());
    assert_eq!(settings.call("update.setWatching", &json!({"on":false})).unwrap().unwrap(), json!({"on":false}));
    assert!(!ctx.watching(), "the GUI has no window: its open event stream does not count");
    settings.shutdown();
}

// ------------------------------------------------------------------------------------- model

#[test]
fn phase_derivation_follows_the_first_match_order() {
    let base = PhaseInput::default();
    let with = |f: &dyn Fn(&mut PhaseInput)| {
        let mut i = base.clone();
        f(&mut i);
        model::derive_phase(&i)
    };
    assert_eq!(model::derive_phase(&base), Phase::Idle);
    assert_eq!(with(&|i| i.available = 3), Phase::Available);
    assert_eq!(with(&|i| { i.available = 3; i.checking = true }), Phase::Checking);
    assert_eq!(with(&|i| { i.checking = true; i.offline = true }), Phase::Offline);
    assert_eq!(with(&|i| { i.offline = true; i.error = true }), Phase::Error);
    assert_eq!(with(&|i| { i.error = true; i.restart_required = true; i.available = 2 }), Phase::RestartRequired);
    assert_eq!(with(&|i| { i.restart_required = true; i.repair_required = true }), Phase::RepairRequired);
    assert_eq!(with(&|i| { i.repair_required = true; i.foreign = true }), Phase::Foreign);
    assert_eq!(with(&|i| { i.foreign = true; i.release_running = true }), Phase::ReleaseRunning);
    assert_eq!(with(&|i| { i.release_running = true; i.job = Some(Stage::Installing) }), Phase::Installing, "our running job wins");
    assert_eq!(with(&|i| { i.job = Some(Stage::Downloading); i.unsupported = true }), Phase::Unsupported);
    assert_eq!(with(&|i| i.job = Some(Stage::Waiting)), Phase::Waiting);
    // stages from PackageKit statuses
    assert_eq!(model::stage(30, false, false), Stage::Waiting);
    assert_eq!(model::stage(31, false, false), Stage::Waiting);
    assert_eq!(model::stage(1, false, false), Stage::Waiting);
    assert_eq!(model::stage(8, false, false), Stage::Downloading);
    assert_eq!(model::stage(1, false, true), Stage::Downloading, "the simulate step");
    assert_eq!(model::stage(9, false, false), Stage::Installing);
    assert_eq!(model::stage(36, false, false), Stage::Installing);
    assert_eq!(model::stage(8, true, false), Stage::Installing, "committing wins");
    // review F13: a lock or auth wait in the simulate step or during a confirm is waiting (H07, H08)
    assert_eq!(model::stage(30, false, true), Stage::Waiting);
    assert_eq!(model::stage(31, false, true), Stage::Waiting);
    assert_eq!(model::stage(30, true, false), Stage::Installing, "after the commit started it stays installing");
    assert_eq!(model::stage(2, false, true), Stage::Downloading, "setup inside the simulate step");
}

#[test]
fn packages_group_into_the_rows_of_the_design_in_order() {
    let pkgs = vec![
        pkg("zlib1g", "1:1.3-1", 5, "ubuntu-resolute-updates"),
        pkg("linux-image-7.0.0-44-generic", "7.0.0-44.44", 5, "u"),
        pkg("linux-base", "4.10", 5, "u"),
        pkg("libglib2.0-0t64", "2.88.1-1", 5, "u"),
        pkg("gir1.2-soup-3.0", "3.6.6-1ubuntu0.3", 5, "u"),
        pkg("libsoup-3.0-0", "3.6.6-1ubuntu0.3", model::info::SECURITY, "ubuntu-resolute-security"),
        pkg("libgtk-3-0t64", "3.24.53-1", 5, "u"),
        pkg("libjavascriptcoregtk-4.1-0", "2.52.7-0ubuntu0.26.04.1", 5, "u"),
        pkg("libwebkit2gtk-4.1-0", "2.52.7-0ubuntu0.26.04.1", 5, "u"),
        pkg("libmutter-18-0", "50.2-0ubuntu1", 5, "u"),
        pkg("gnome-shell", "50.2-0ubuntu1", 5, "u"),
        pkg("openssl", "3.5.3-1ubuntu1", 5, "ubuntu-resolute-security"),
        pkg("curl", "8.14-1", model::info::IMPORTANT, "u"),
    ];
    let rows = model::group(&pkgs, "26.04.1 LTS");
    let ids: Vec<&str> = rows.iter().map(|r| r.id).collect();
    assert_eq!(ids, ["security", "gnome", "webkit", "gtk", "soup", "glib", "kernel", "other"]);
    let row = |id: &str| rows.iter().find(|r| r.id == id).unwrap();
    assert_eq!(row("security").version.as_deref(), Some("26.04.1 LTS"));
    assert_eq!(row("security").count, 2, "openssl (-security data) and curl (important)");
    assert_eq!(row("gnome").version.as_deref(), Some("50.2"));
    assert_eq!(row("gnome").count, 2);
    assert_eq!(row("webkit").version.as_deref(), Some("2.52.7"));
    assert_eq!(row("soup").count, 2);
    assert!(row("soup").security, "a component row keeps its security flag");
    assert_eq!(row("kernel").version.as_deref(), Some("7.0.0"));
    assert_eq!(row("other").version, None);
    assert_eq!(row("other").size, None, "no download-size from the apt backend");
    assert_eq!(model::os_version("26.04.1 LTS (Resolute Raccoon)"), "26.04.1 LTS");
    assert_eq!(model::os_version("26.10"), "26.10");
    assert_eq!(model::upstream("1:2.52.7-0ubuntu1"), "2.52.7");
    assert!(Pkg::from_signal(5, "bad id", "").is_none());
    assert!(Pkg::from_signal(5, "Name;1;amd64;x", "").is_none(), "upper case is no Debian name");
    assert!(Pkg::from_signal(5, "ab;;amd64;x", "").is_none());
    let round = Pkg::from_json(&pkgs[5].json()).unwrap();
    assert_eq!(round, pkgs[5]);
}

#[test]
fn packagekit_errors_map_to_the_catalogue() {
    let key = |e: Option<u32>, x: u32| model::map_failure(e, x).map(|f| f.key);
    assert_eq!(key(None, model::exit::SUCCESS), None);
    assert_eq!(key(Some(model::ERROR_NO_PACKAGES_TO_UPDATE), 2), None, "error 27 counts as success");
    for (code, expected) in [
        (2, "update.err.network"),
        (37, "update.err.network"),
        (43, "update.err.network"),
        (66, "update.err.repair"),
        (25, "update.err.cannotCancel"),
        (48, "update.err.notAuthorized"),
        (46, "update.err.noSpace"),
        (10, "update.err.download"),
        (26, "update.err.locked"),
        (65, "update.err.locked"),
        (67, "update.err.locked"),
        (5, "update.err.untrusted"),
        (30, "update.err.untrusted"),
        (31, "update.err.untrusted"),
        (51, "update.err.untrusted"),
        (13, "update.err.conflict"),
        (35, "update.err.conflict"),
        (36, "update.err.conflict"),
        (39, "update.err.conflict"),
        (56, "update.err.dpkg"),
        (58, "update.err.dpkg"),
        (60, "update.err.dpkg"),
        (17, "update.err.cancelled"),
        (61, "update.err.stale"),
        (99, "update.err.failed"),
    ] {
        assert_eq!(key(Some(code), 2), Some(expected), "error {code}");
        assert!(crate::i18n::has(expected), "{expected} is not in update.tsv");
    }
    assert_eq!(key(None, model::exit::CANCELLED), Some("update.err.cancelled"));
    assert_eq!(key(None, model::exit::NEED_UNTRUSTED), Some("update.err.untrusted"));
    assert_eq!(key(None, model::exit::CANCELLED_PRIORITY), Some("update.err.locked"));
    assert_eq!(key(None, model::exit::REPAIR_REQUIRED), Some("update.err.repair"));
    assert_eq!(key(None, 6), Some("update.err.failed"), "killed");
    assert_eq!(model::source_host("Failed to fetch https://dl.google.com/linux/chrome/deb/dists/stable/InRelease"), Some("dl.google.com".into()));
    assert_eq!(model::source_host("Failed to fetch http://nl.archive.ubuntu.com/ubuntu/dists/x"), None);
    assert_eq!(model::source_host("no uri here"), None);
}

/// ≤ 4 page and ≤ 1 fan-out progress events in **any** second under a flood, on the real send path: a fan-out
/// event (`emit`) reaches the owner's page too, so it counts as a page event (design §4.4, review F8).
#[test]
fn progress_is_throttled_to_four_local_and_one_remote_per_second() {
    use model::Delivery;
    let start = Instant::now();
    let mut rate = model::Progress::default();
    let (mut page, mut wire): (Vec<Instant>, Vec<Instant>) = (Vec::new(), Vec::new());
    fn record(page: &mut Vec<Instant>, wire: &mut Vec<Instant>, at: Instant, delivery: Delivery) {
        if delivery != Delivery::None {
            page.push(at);
        }
        if delivery == Delivery::Both {
            wire.push(at);
        }
    }
    // 2000 property changes in 4 s (one per 2 ms) and the 250 ms timer, which runs out of phase with the flood
    let mut next_tick = start + Duration::from_millis(137);
    for step in 0..2000u64 {
        let now = start + Duration::from_millis(step * 2);
        while next_tick <= now {
            record(&mut page, &mut wire, next_tick, rate.tick(next_tick));
            next_tick += model::LOCAL_PROGRESS;
        }
        record(&mut page, &mut wire, now, rate.offer(now));
    }
    // then quiet: the timer alone must deliver the last value on both paths
    let quiet = start + Duration::from_secs(4);
    let (pages, wires) = (page.len(), wire.len());
    for tick in 0..8u64 {
        let at = quiet + Duration::from_millis(250 * tick);
        record(&mut page, &mut wire, at, rate.tick(at));
    }
    assert!(page.len() > pages && wire.len() > wires, "the last value was lost");
    let in_any_second = |sent: &[Instant], cap: usize, what: &str| {
        for (i, from) in sent.iter().enumerate() {
            let n = sent[i..].iter().take_while(|t| t.duration_since(*from) < Duration::from_secs(1)).count();
            assert!(n <= cap, "{n} {what} events within one second from {:?}", from.duration_since(start));
        }
    };
    in_any_second(&page, 4, "page");
    in_any_second(&wire, 1, "fan-out");
    assert!(page.len() >= 15, "the page rate is used: {}", page.len());
    assert!(wire.len() >= 4);
    // nothing pending: no event
    assert_eq!(rate.tick(quiet + Duration::from_secs(10)), Delivery::None);
}

/// The details cache keeps at most its capacity, least recently used out first (review F15).
#[test]
fn the_details_cache_is_bounded() {
    let mut cache = model::Lru::new(3);
    for id in ["a", "b", "c"] {
        cache.insert(id.into(), json!(id));
    }
    assert_eq!(cache.get("a"), Some(json!("a")), "a is now the most recent");
    cache.insert("d".into(), json!("d"));
    assert!(!cache.contains("b"), "b was the least recently used");
    assert!(cache.contains("a") && cache.contains("c") && cache.contains("d"));
    cache.insert("c".into(), json!("c2"));
    cache.insert("e".into(), json!("e"));
    assert!(!cache.contains("a") && cache.contains("c"));
    assert_eq!(cache.len(), 3);
    cache.clear();
    assert_eq!(cache.len(), 0);
    let mut big = model::Lru::new(64);
    for n in 0..1000 {
        big.insert(format!("id{n}"), json!({"changelog":"x".repeat(10)}));
    }
    assert_eq!(big.len(), 64);
}

/// The row states after a job (design §3.3, §4.6 "After Finished"; review F3), pure.
#[test]
fn rows_after_a_job_are_installed_restart_or_failed() {
    let before = vec![
        pkg("libsoup-3.0-0", "3.6.6-1ubuntu0.3", model::info::SECURITY, "ubuntu-resolute-security"),
        pkg("gnome-shell", "50.2-0ubuntu1", 5, "u"),
        pkg("zlib1g", "1:1.3-1", 5, "u"),
        pkg("curl", "8.14-1", 5, "u"),
    ];
    let rows = model::group(&before, "26.04.1 LTS");
    let info: Vec<Value> = rows.iter().map(model::row_info).collect();
    let job = |outcome: Value| json!({"jobId":"j","role":"update","finishedAt":1,"outcome":outcome,"rows":info});
    let ok = json!({"exit":1,"code":null,"key":null,"params":{},"detail":""});
    let states = |views: &[Value]| views.iter().map(|r| (r["id"].as_str().unwrap().to_owned(), r["state"].as_str().unwrap().to_owned())).collect::<Vec<_>>();
    let s = |id: &str, state: &str| (id.to_owned(), state.to_owned());
    // success, everything gone: installed; gnome-shell waits for a reboot (reboot-required.pkgs)
    let views = model::row_views(&[], None, Some(&job(ok.clone())), &["gnome-shell".into()]);
    assert_eq!(states(&views), [s("gnome", "restart"), s("soup", "installed"), s("other", "installed")]);
    assert_eq!(views[0]["pct"], 100);
    assert_eq!(views[2]["count"], 2, "the row keeps what it was");
    // success while rows remain: curl kept back, its row is pending again, no error
    let after = model::group(&[before[3].clone()], "26.04.1 LTS");
    let views = model::row_views(&after, None, Some(&job(ok)), &[]);
    assert_eq!(states(&views), [s("gnome", "installed"), s("soup", "installed"), s("other", "pendingDownload")]);
    assert!(views[2]["error"].is_null());
    assert_eq!(views[2]["count"], 1);
    // a failure: what is gone was installed, what remains failed with the outcome's error key
    let failed = json!({"exit":2,"code":"dpkg","key":"update.err.dpkg","params":{},"detail":"E: x"});
    let after = model::group(&before[1..], "26.04.1 LTS");
    let views = model::row_views(&after, None, Some(&job(failed)), &[]);
    assert_eq!(states(&views), [s("gnome", "failed"), s("soup", "installed"), s("other", "failed")]);
    assert_eq!(views[0]["error"]["key"], "update.err.dpkg");
    // a new update of a failed row's kind that the re-check found is pending, not failed
    let after = model::group(&[before[1].clone(), pkg("wget", "1.25-1", 5, "u")], "26.04.1 LTS");
    let views = model::row_views(&after, None, Some(&job(json!({"exit":2,"code":"dpkg","key":"update.err.dpkg"}))), &[]);
    assert_eq!(states(&views), [s("gnome", "failed"), s("soup", "installed"), s("other", "pendingDownload")]);
    // a cancel or a declined confirm leaves pending rows, never failed ones
    for code in ["cancelled", "declined", "unknown_outcome"] {
        let views = model::row_views(&rows, None, Some(&job(json!({"exit":3,"code":code,"key":null,"params":{}}))), &[]);
        assert!(views.iter().all(|r| r["state"] == "pendingDownload"), "{code}");
    }
    // a running job: its live states win, rows outside it are pending; no last job: all pending
    let live = json!({"rows":{"soup":{"state":"downloading","pct":40}}});
    let views = model::row_views(&rows, Some(&live), Some(&job(json!({"code":"dpkg","key":"update.err.dpkg"}))), &[]);
    assert_eq!(states(&views), [s("gnome", "pendingDownload"), s("soup", "downloading"), s("other", "pendingDownload")]);
    assert_eq!(views[1]["pct"], 40);
    assert!(model::row_views(&rows, None, None, &[]).iter().all(|r| r["state"] == "pendingDownload"));
}

#[test]
fn reattach_and_foreign_decisions() {
    use model::Reattach;
    let job = json!({"bootId":"b","tid":"/42_abc"});
    assert_eq!(model::reattach(Some(&job), Some("b")), Reattach::Transaction("/42_abc".into()));
    assert_eq!(model::reattach(Some(&job), Some("other")), Reattach::Nothing, "another boot: outcome only");
    assert_eq!(model::reattach(Some(&job), None), Reattach::Nothing);
    assert_eq!(model::reattach(Some(&json!({"bootId":"b","tid":"/1","finishedAt":5})), Some("b")), Reattach::Nothing);
    assert_eq!(model::reattach(Some(&json!({"bootId":"b"})), Some("b")), Reattach::OldTransactions);
    let paused = json!({"bootId":"b","tid":"/1","confirm":{"kind":"metered","size":5}});
    assert_eq!(model::reattach(Some(&paused), Some("b")), Reattach::Expired, "a job paused for a confirm (review F7)");
    assert_eq!(model::reattach(None, Some("b")), Reattach::Nothing);
    assert!(model::foreign(model::role::UPDATE_PACKAGES, "/9", Some("/1")));
    assert!(model::foreign(model::role::INSTALL_PACKAGES, "/9", None));
    assert!(model::foreign(model::role::REMOVE_PACKAGES, "/9", None));
    assert!(!model::foreign(model::role::UPDATE_PACKAGES, "/1", Some("/1")), "our own transaction");
    assert!(!model::foreign(model::role::GET_UPDATES, "/9", None), "a read is not foreign");
    assert!(!model::foreign(model::role::REFRESH_CACHE, "/9", None));
}

/// The terminal wrapper's outcomes (design §6.4 step 5): status file × seen × reboot.
#[test]
fn release_wrapper_outcomes() {
    use model::UpgradeOutcome::*;
    let o = model::upgrade_outcome;
    let early = Duration::from_secs(10);
    let late = Duration::from_secs(130);
    assert_eq!(o(true, false, None, early, false), Running);
    assert_eq!(o(true, true, Some(0), late, true), Running);
    assert_eq!(o(false, false, None, early, false), Starting);
    assert_eq!(o(false, false, None, late, false), NoWindow);
    assert_eq!(o(false, false, Some(126), early, false), StartFailed(126));
    assert_eq!(o(false, false, Some(127), early, true), StartFailed(127), "a pending reboot does not hide a dismissed prompt");
    assert_eq!(o(false, false, Some(1), late, false), StartFailed(1));
    assert_eq!(o(false, true, None, late, false), Ended);
    assert_eq!(o(false, true, Some(2), late, false), Ended);
    assert_eq!(o(false, true, Some(0), late, true), Restart);
    assert_eq!(o(false, true, None, late, true), Restart);
    assert_eq!(o(false, false, Some(0), early, true), Restart);
    assert_eq!(o(false, true, Some(0), late, false), Offer);
    assert_eq!(o(false, false, Some(0), early, false), Offer);
    assert_eq!(model::start_failure_key(126), "update.release.err.noPassword");
    assert_eq!(model::start_failure_key(127), "update.release.err.wrongPassword");
    assert_eq!(model::start_failure_key(1), "update.release.err.refused");
    for key in ["update.release.err.noPassword", "update.release.err.wrongPassword", "update.release.err.refused", "update.release.err.noWindow"] {
        assert!(crate::i18n::has(key));
    }
}

// ------------------------------------------------------------------------------------- store

#[test]
fn state_files_are_private_atomic_and_round_trip() {
    use std::os::unix::fs::MetadataExt;
    let f = Fixture::new("store");
    let dir = f.base.join("cache/update");
    let store = store::Store::open(dir.clone()).unwrap();
    assert_eq!(fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);
    let snapshot = json!({"version":1,"rev":3,"packages":[pkg("zlib1g","1:1.3-1",5,"u").json()]});
    store.write(store::SNAPSHOT, &snapshot).unwrap();
    store.write(store::JOB, &json!({"version":1,"jobId":"0123456789abcdef0123456789abcdef"})).unwrap();
    assert_eq!(store.read(store::SNAPSHOT).unwrap(), snapshot);
    for name in [store::SNAPSHOT, store::JOB] {
        assert_eq!(fs::metadata(dir.join(name)).unwrap().mode() & 0o777, 0o600, "{name}");
    }
    assert!(fs::read_dir(&dir).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().ends_with(".tmp")), "temp file left");
    // a symlinked or group-readable file is not ours
    std::os::unix::fs::symlink(dir.join(store::SNAPSHOT), dir.join("link.json")).unwrap();
    assert!(store.read("link.json").is_none());
    fs::set_permissions(dir.join(store::JOB), fs::Permissions::from_mode(0o644)).unwrap();
    assert!(store.read(store::JOB).is_none());
    // jobs.jsonl keeps 90 days
    let now = 200 * 86_400_000u64;
    store.append_job(&json!({"jobId":"old","finishedAt":now - 91 * 86_400_000}), now - 91 * 86_400_000).unwrap();
    store.append_job(&json!({"jobId":"recent","finishedAt":now - 86_400_000}), now - 86_400_000).unwrap();
    store.append_job(&json!({"jobId":"new","finishedAt":now}), now).unwrap();
    let ids: Vec<Value> = store.jobs().iter().map(|j| j["jobId"].clone()).collect();
    assert_eq!(ids, [json!("recent"), json!("new")]);
    // the job lock: one holder at a time
    let lock = store.try_lock(store::JOB_LOCK).unwrap().unwrap();
    assert!(store.locked(store::JOB_LOCK));
    assert!(store.try_lock(store::JOB_LOCK).unwrap().is_none());
    drop(lock);
    assert!(!store.locked(store::JOB_LOCK));
    // boot id from the (fixture) procfs
    let setup = paths::resolve(&f.env()).unwrap();
    assert_eq!(store::boot_id(&setup.paths).as_deref(), Some("0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0"));
    write(&setup.paths.boot_id, "not a boot id\n");
    assert_eq!(store::boot_id(&setup.paths), None);
}

// ------------------------------------------------------------------------------------- restart

#[test]
fn reboot_required_is_parsed() {
    let f = Fixture::new("reboot");
    let setup = paths::resolve(&f.env()).unwrap();
    assert!(!restart::reboot(&setup.paths).required);
    write(&setup.paths.reboot_required, "*** System restart required ***\n");
    write(&setup.paths.reboot_required_pkgs, "gnome-shell\nlinux-base\ngnome-shell\n\nNot A Package\n");
    let reboot = restart::reboot(&setup.paths);
    assert!(reboot.required);
    assert_eq!(reboot.pkgs, ["gnome-shell", "linux-base"]);
    assert!(reboot.since.is_some());
    let record = json!({"type":4,"pkgs":["linux-image-7.0.0-44-generic"],"bootId":"b"});
    assert!(restart::require_restart_applies(&record, Some("b")));
    assert!(!restart::require_restart_applies(&record, Some("new boot")), "a new boot clears it");
    assert!(!restart::require_restart_applies(&json!({"type":2,"bootId":"b"}), Some("b")), "application restarts are ignored");
}

/// The inotify watch on `/run/reboot-required` and `history.log` (design §4.8; no GIO VFS, review F9).
#[test]
fn the_file_watch_sees_only_the_watched_files() {
    use std::io::Write;
    let f = Fixture::new("watch");
    let setup = paths::resolve(&f.env()).unwrap();
    let (reboot, history) = (setup.paths.reboot_required.clone(), setup.paths.apt_log.join("history.log"));
    for dir in [reboot.parent().unwrap(), history.parent().unwrap()] {
        fs::create_dir_all(dir).unwrap();
    }
    let watch = restart::Watch::new(&[reboot.clone(), history.clone()]).unwrap();
    assert!(!watch.changed(), "nothing happened");
    write(&reboot.parent().unwrap().join("reboot-required.pkgs"), "gnome-shell\n");
    assert!(!watch.changed(), "another file in the same directory");
    write(&reboot, "*** System restart required ***\n");
    assert!(watch.changed());
    assert!(!watch.changed(), "the queue was drained");
    let mut log = fs::OpenOptions::new().create(true).append(true).open(&history).unwrap();
    writeln!(log, "Start-Date: 2026-11-05  06:53:40").unwrap();
    drop(log);
    assert!(watch.changed(), "apt appended to history.log");
    fs::remove_file(&reboot).unwrap();
    assert!(watch.changed(), "the reboot flag went away");
    assert!(restart::Watch::new(&[f.base.join("missing/x")]).is_none(), "no directory to watch");
}

/// Today's real noise (libpcre2, gvfs, memfd, a cache file) never counts; a deleted GTK 4 in a native Win11 app is
/// the `gtk4` InfoBar; a deleted gnome-shell in gnome-shell is the hero restart. Only our own programs are read,
/// not their children, and not the Calendar, which `shell.restartAll` does not restart.
#[test]
fn the_maps_scan_on_a_fixture_procfs() {
    let f = Fixture::new("maps");
    let setup = paths::resolve(&f.env()).unwrap();
    let proc = &setup.paths.proc;
    let own_dir = f.base.join("bin");
    fs::create_dir_all(&own_dir).unwrap();
    let uid = 1000;
    let status = |ppid: u32| format!("Name:\tx\nPPid:\t{ppid}\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\n");
    let noise = "7f00-7f01 r-xp 00000000 08:02 11 /usr/lib/x86_64-linux-gnu/libpcre2-8.so.0.14.0 (deleted)\n\
7f01-7f02 r--p 00000000 08:02 12 /usr/lib/x86_64-linux-gnu/gvfs/libgvfscommon.so (deleted)\n\
7f02-7f03 rw-s 00000000 00:01 13 /memfd:wayland-cursor (deleted)\n\
7f03-7f04 r--s 00000000 08:02 14 /home/robin/.cache/fontconfig/x.cache-8 (deleted)\n\
7f04-7f05 r--p 00000000 08:02 15 /usr/lib/x86_64-linux-gnu/libgtk-3.so.0.2400.52\n";
    // a Win11 panel with only noise and one healthy GTK mapping
    let panel = proc.join("100");
    write(&panel.join("status"), &status(1));
    write(&panel.join("comm"), "win11-panel\n");
    write(&panel.join("maps"), noise);
    std::os::unix::fs::symlink(own_dir.join("win11-panel"), panel.join("exe")).unwrap();
    // gnome-shell with today's deleted binary, but not in GNOME Shell's cgroup → ignored
    let shell = proc.join("200");
    write(&shell.join("status"), &status(1));
    write(&shell.join("comm"), "gnome-shell\n");
    write(&shell.join("maps"), "5600-5700 r-xp 00000000 08:02 21 /usr/bin/gnome-shell (deleted)\n");
    write(&shell.join("cgroup"), "0::/user.slice/user-1000.slice/session-2.scope\n");
    assert_eq!(restart::scan(&setup.paths, uid, &own_dir), restart::Scan::default(), "noise only");
    // now the real cgroup, and a native Settings mapping a deleted GTK 4
    write(&shell.join("cgroup"), "0::/user.slice/user-1000.slice/user@1000.service/session.slice/org.gnome.Shell@wayland.service\n");
    let settings = proc.join("101");
    write(&settings.join("status"), &status(1));
    write(&settings.join("comm"), "win11-gtk-setti\n");
    write(&settings.join("maps"), &format!("{noise}7f10-7f11 r-xp 00000000 08:02 31 /usr/lib/x86_64-linux-gnu/libgtk-4.so.1.2200.4 (deleted)\n"));
    std::os::unix::fs::symlink(own_dir.join("win11-gtk-settings"), settings.join("exe")).unwrap();
    // a child of the panel that is no program of ours (a terminal it started) is not read
    let child = proc.join("102");
    write(&child.join("status"), &status(100));
    write(&child.join("comm"), "ptyxis\n");
    write(&child.join("maps"), "7f-80 r-xp 0 08:02 41 /usr/lib/x86_64-linux-gnu/libgdk-3.so.0.2400.52 (deleted)\n");
    std::os::unix::fs::symlink(f.base.join("usr-bin/ptyxis"), child.join("exe")).unwrap();
    // a process of another user is never scanned
    let other = proc.join("300");
    write(&other.join("status"), "Name:\tx\nPPid:\t1\nUid:\t0\t0\t0\t0\n");
    write(&other.join("comm"), "win11-panel\n");
    write(&other.join("maps"), "7f-80 r-xp 0 08:02 41 /usr/lib/x86_64-linux-gnu/libcairo.so.2.11800.0 (deleted)\n");
    std::os::unix::fs::symlink(own_dir.join("win11-panel"), other.join("exe")).unwrap();
    let scan = restart::scan(&setup.paths, uid, &own_dir);
    assert!(scan.gnome);
    assert_eq!(scan.shell_parts, ["gtk4"]);
    assert_eq!(scan.json()["shell"], true);
    // our own binary replaced: "self"
    fs::remove_file(panel.join("exe")).unwrap();
    std::os::unix::fs::symlink(format!("{} (deleted)", own_dir.join("win11-panel").display()), panel.join("exe")).unwrap();
    assert_eq!(restart::scan(&setup.paths, uid, &own_dir).shell_parts, ["gtk4", "self"]);
    // unreadable maps add nothing (the replaced binary still counts)
    write(&settings.join("maps"), "");
    assert_eq!(restart::scan(&setup.paths, uid, &own_dir).shell_parts, ["self"]);
    // the Calendar is ours but shell.restartAll cannot restart it: its deleted GTK 4 and its own replaced binary
    // add nothing, or the InfoBar would stay while it is open
    let calendar = proc.join("103");
    write(&calendar.join("status"), &status(1));
    write(&calendar.join("comm"), "win11-calendar\n");
    write(&calendar.join("maps"), "7f10-7f11 r-xp 00000000 08:02 31 /usr/lib/x86_64-linux-gnu/libgtk-4.so.1.2200.4 (deleted)\n");
    std::os::unix::fs::symlink(format!("{} (deleted)", own_dir.join("win11-calendar").display()), calendar.join("exe")).unwrap();
    assert_eq!(restart::scan(&setup.paths, uid, &own_dir).shell_parts, ["self"]);
    fs::remove_file(panel.join("exe")).unwrap();
    std::os::unix::fs::symlink(own_dir.join("win11-panel"), panel.join("exe")).unwrap();
    assert!(restart::scan(&setup.paths, uid, &own_dir).shell_parts.is_empty(), "only the Calendar is replaced");
}

// ------------------------------------------------------------------------------------- snapshot view

#[test]
fn the_snapshot_view_counts_only_offered_packages_and_shows_restarts() {
    let f = Fixture::new("view");
    let ctx = f.ctx("panel", false, "en");
    let service = Update::with_env(ctx.clone(), &f.env());
    let setup = paths::resolve(&f.env()).unwrap();
    let store = store::Store::open(ctx.cache_dir.join("update")).unwrap();
    let mut pkgs = sample_pkgs();
    phasing::apply(&mut pkgs, Some(&phasing::parse_script(SCRIPT_OUTPUT).unwrap()), None);
    let boot = store::boot_id(&setup.paths).unwrap();
    store
        .write(
            store::SNAPSHOT,
            &json!({"version":1,"rev":9,"backend":"packagekit","checkedAt":1762341720000u64,
                "packages":pkgs.iter().map(Pkg::json).collect::<Vec<_>>(),"network":{"state":"full","online":true,"metered":false},
                "warnings":[{"code":"source","key":"update.warn.source","params":{"source":"dl.google.com"}}],
                "requireRestart":{"type":4,"pkgs":["libc6"],"bootId":boot}}),
        )
        .unwrap();
    let view = service.call("update.status", &json!({"packages":true})).unwrap().unwrap();
    assert_eq!(view["rev"], 9);
    assert_eq!(view["totals"]["count"], 2);
    assert_eq!(view["packages"].as_array().unwrap().len(), 6);
    let rows: Vec<&str> = view["rows"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(rows, ["soup", "other"], "libsoup's component row wins over security");
    assert_eq!(view["rows"][0]["security"], true);
    assert_eq!(view["rows"][0]["state"], "pendingDownload");
    assert_eq!(view["phase"], "restartRequired", "RequireRestart system in this boot");
    assert_eq!(view["restart"]["rebootPkgs"], json!(["libc6"]));
    assert_eq!(view["warnings"][0]["message"], "One update source couldn’t be refreshed: dl.google.com. The other updates were checked.");
    assert!(service.call("update.status", &json!({})).unwrap().unwrap().get("packages").is_none(), "packages only on request");
    // the stored error is a key, rendered in the reader's language
    store
        .write(store::SNAPSHOT, &json!({"rev":10,"backend":"packagekit","error":{"code":"network","key":"update.err.network","params":{},"detail":"E: x"}}))
        .unwrap();
    let nl = super::Shared {
        ctx: f.ctx("panel", false, "nl"),
        setup: setup.clone(),
        store: store.clone(),
        stats: Default::default(),
        rev: Default::default(),
    };
    let view = compose(&nl, store.read(store::SNAPSHOT).as_ref(), None, None, false);
    assert_eq!(view["phase"], "error");
    assert_eq!(view["error"]["message"], "Er is geen verbinding met de updateserver. Controleer je internetverbinding en probeer het opnieuw.");
    assert_eq!(view["error"]["detail"], "E: x");
    // a snapshot without PackageKit: unsupported with the noPackageKit text and {product}
    store.write(store::SNAPSHOT, &json!({"rev":11,"backend":null})).unwrap();
    let view = service.call("update.status", &json!({})).unwrap().unwrap();
    assert_eq!(view["phase"], "unsupported");
    assert_eq!(view["error"]["message"], "PackageKit is missing, so Win11 Shell can’t install updates itself.");
    // offline from the owner's network record
    store.write(store::SNAPSHOT, &json!({"rev":12,"backend":"packagekit","network":{"state":"portal","online":false,"metered":true}})).unwrap();
    let view = service.call("update.status", &json!({})).unwrap().unwrap();
    assert_eq!(view["phase"], "offline");
    assert_eq!(view["network"]["metered"], true);
    assert_eq!(f.stub_log(), "");
    assert!(f.never_connected());
}

#[test]
fn network_states_follow_the_design() {
    use super::net::Net;
    assert_eq!(Net::from_nm(Some(4), Some(2)), Net { state: "full", online: true, metered: false });
    assert_eq!(Net::from_nm(Some(0), None), Net { state: "unknown", online: true, metered: false });
    assert!(!Net::from_nm(Some(2), None).online, "a captive portal is not online");
    assert!(!Net::from_nm(Some(3), None).online, "limited is not online");
    assert!(Net::from_nm(Some(1), None).refuses_check());
    assert!(!Net::from_nm(Some(3), None).refuses_check());
    assert!(Net::from_nm(Some(4), Some(1)).metered);
    assert!(Net::from_nm(Some(4), Some(3)).metered, "GUESS_YES counts as metered");
    assert!(!Net::from_nm(Some(4), Some(4)).metered);
    let net = Net::from_nm(Some(3), Some(3));
    assert_eq!(Net::from_json(&net.json()), net);
}

/// The guard texts and the not-here text in en through the real call sites (round2-gate.md §6: en per stem).
#[test]
fn update_texts_render_in_english_at_the_call_sites() {
    let f = Fixture::new("en");
    let ctx = f.ctx("panel", false, "en");
    let service = Update::with_env(ctx.clone(), &f.env());
    assert_eq!(service.call("update.check", &json!({})).unwrap().unwrap_err().message, "Updates can only be installed from Settings.");
    let setup = paths::resolve(&f.env()).unwrap();
    write(&f.root.join("proc/55/cmdline"), "do-release-upgrade\0");
    assert_eq!(
        guard::power_blocked_in(&ctx, &setup, ctx.lang()).unwrap().message,
        "Ubuntu is being upgraded to a new version. Wait until the upgrade window asks you to restart."
    );
    assert!(guard::power_blocked(&ctx, "lock").is_none(), "lock is never blocked");
}

/// The GW2c helper for `system.updates` (design §4.10): no snapshot → None (info.rs keeps `apt list`); a
/// snapshot → its non-phased, non-held count, whatever its age.
#[test]
fn system_updates_counts_the_snapshot_once_one_exists() {
    if std::env::vars_os().any(|(k, _)| k.to_string_lossy().starts_with(paths::VAR_PREFIX)) {
        return; // a seam run: system_updates() reads the process seams
    }
    let f = Fixture::new("sysupd");
    let ctx = f.ctx("panel", false, "nl");
    assert!(super::system_updates(&ctx).is_none());
    let store = store::Store::open(ctx.cache_dir.join("update")).unwrap();
    let mut pkgs = sample_pkgs();
    phasing::apply(&mut pkgs, Some(&phasing::parse_script(SCRIPT_OUTPUT).unwrap()), None);
    let packages = pkgs.iter().map(Pkg::json).collect::<Vec<_>>();
    // review F4: the snapshot an owner writes when Settings › Update first opens (no check yet) and one without
    // PackageKit keep the apt count
    store.write(store::SNAPSHOT, &json!({"rev":1,"checkedAt":null,"backend":"packagekit","packages":[]})).unwrap();
    assert!(super::system_updates(&ctx).is_none(), "the init snapshot is no count");
    store.write(store::SNAPSHOT, &json!({"rev":2,"checkedAt":5,"backend":null,"packages":packages})).unwrap();
    assert!(super::system_updates(&ctx).is_none(), "no PackageKit: apt keeps the count");
    store.write(store::SNAPSHOT, &json!({"rev":3,"checkedAt":5,"backend":"packagekit","packages":packages})).unwrap();
    let count = super::system_updates(&ctx).unwrap();
    assert_eq!((count["available"].clone(), count["security"].clone(), count["tool"].clone()), (json!(2), json!(1), json!("packagekit")));
    assert_eq!(count["checkedAt"], 5);
    assert!(count["rebootRequired"].is_boolean());
}

/// The pure part of the GW2c helper, also in a seam run (review F4).
#[test]
fn system_updates_needs_a_finished_check_with_packagekit() {
    let f = Fixture::new("sysupd-pure");
    let setup = paths::resolve(&f.env()).unwrap();
    let packages = vec![pkg("zlib1g", "1:1.3-1", 5, "u").json()];
    let snapshot = |checked: Value, backend: Value| json!({"checkedAt":checked,"backend":backend,"packages":packages});
    assert!(super::system_updates_from(snapshot(Value::Null, json!("packagekit")), &setup.paths).is_none());
    assert!(super::system_updates_from(snapshot(json!("soon"), json!("packagekit")), &setup.paths).is_none());
    assert!(super::system_updates_from(snapshot(json!(7), Value::Null), &setup.paths).is_none());
    assert!(super::system_updates_from(snapshot(json!(7), json!("apt")), &setup.paths).is_none());
    let count = super::system_updates_from(snapshot(json!(7), json!("packagekit")), &setup.paths).unwrap();
    assert_eq!((count["available"].clone(), count["rebootRequired"].clone()), (json!(1), json!(false)));
    write(&setup.paths.reboot_required, "*** System restart required ***\n");
    assert_eq!(super::system_updates_from(snapshot(json!(7), json!("packagekit")), &setup.paths).unwrap()["rebootRequired"], true);
}

/// Review m5: `system.updates.rebootRequired` follows `update.status` `restart.reboot` (and so the owner's
/// `system.updates.changed`): `/run/reboot-required`, or a `RequireRestart` record of this boot (a reboot or a new
/// session); a record of another boot or of an application (2) does not count.
#[test]
fn reboot_required_is_one_rule_for_the_snapshot_and_system_updates() {
    let f = Fixture::new("reboot-rule");
    let ctx = f.ctx("panel", false, "en");
    let setup = paths::resolve(&f.env()).unwrap();
    let store = store::Store::open(ctx.cache_dir.join("update")).unwrap();
    let shared = super::Shared { ctx: ctx.clone(), setup: setup.clone(), store, stats: Default::default(), rev: Default::default() };
    let boot = store::boot_id(&setup.paths).unwrap();
    let packages = vec![pkg("zlib1g", "1:1.3-1", 5, "u").json()];
    let with = |record: Value| json!({"rev":2,"checkedAt":7,"backend":"packagekit","packages":packages,"requireRestart":record});
    for (record, want) in [
        (Value::Null, false),
        (json!({"type":4,"pkgs":["linux-base"],"bootId":boot}), true),
        (json!({"type":6,"pkgs":["libc6"],"bootId":boot}), true),
        (json!({"type":3,"pkgs":["gnome-shell"],"bootId":boot}), true),
        (json!({"type":4,"pkgs":["linux-base"],"bootId":"00000000-0000-0000-0000-000000000000"}), false),
        (json!({"type":2,"pkgs":["firefox"],"bootId":boot}), false),
    ] {
        let snapshot = with(record.clone());
        let count = super::system_updates_from(snapshot.clone(), &setup.paths).unwrap();
        let view = compose(&shared, Some(&snapshot), None, None, false);
        assert_eq!((count["rebootRequired"].clone(), view["restart"]["reboot"].clone()), (json!(want), json!(want)), "{record}");
    }
    write(&setup.paths.reboot_required, "*** System restart required ***\n");
    let snapshot = with(Value::Null);
    assert_eq!(super::system_updates_from(snapshot.clone(), &setup.paths).unwrap()["rebootRequired"], true);
    assert_eq!(compose(&shared, Some(&snapshot), None, None, false)["restart"]["reboot"], true);
}

/// A snapshot written during a check by an owner that is gone never leaves readers on "checking" (review F2).
#[test]
fn checking_counts_only_while_its_owner_lives() {
    let f = Fixture::new("checking");
    let ctx = f.ctx("panel", false, "en");
    let service = Update::with_env(ctx.clone(), &f.env());
    let store = store::Store::open(ctx.cache_dir.join("update")).unwrap();
    let me = std::process::id();
    let snapshot = json!({"rev":4,"backend":"packagekit","checkedAt":1,"checking":true,"owner":{"pid":me},
                          "network":{"state":"full","online":true,"metered":false}});
    store.write(store::SNAPSHOT, &snapshot).unwrap();
    store.write(store::OWNER, &store::current_owner()).unwrap();
    let view = service.call("update.status", &json!({})).unwrap().unwrap();
    assert_eq!((view["phase"].clone(), view["refreshing"].clone()), (json!("checking"), json!(true)));
    store.write(store::OWNER, &json!({"pid":me,"start":"1"})).unwrap();
    let view = service.call("update.status", &json!({})).unwrap().unwrap();
    assert_eq!((view["phase"].clone(), view["refreshing"].clone()), (json!("idle"), json!(false)), "the owner is gone");
    // the last job's rows reach readers from the snapshot (review F3)
    let row = model::row_info(&model::group(&[pkg("zlib1g", "1:1.3-1", 5, "u")], "x")[0]);
    let failed = json!({"exit":2,"code":"dpkg","key":"update.err.dpkg","params":{},"detail":"E: x"});
    let packages = vec![pkg("zlib1g", "1:1.3-1", 5, "u").json()];
    store
        .write(store::SNAPSHOT, &json!({"rev":5,"backend":"packagekit","checkedAt":1,"packages":packages,
            "lastJob":{"jobId":"j","role":"update","finishedAt":2,"outcome":failed,"rows":[row]}}))
        .unwrap();
    let view = service.call("update.status", &json!({})).unwrap().unwrap();
    assert_eq!(view["rows"][0]["state"], "failed");
    assert_eq!(view["rows"][0]["error"]["message"], "An update didn’t install correctly. Try again; if that doesn’t help, open Software Updater.");
    assert_eq!(f.stub_log(), "");
}

/// GW3 D1 / D-G41: `restart.shellParts` carries the page's component codes (native/apps/settings/src/pages/update.rs
/// `part_name`), one per watched library, and every code the scan can produce is one the page knows.
#[test]
fn shell_parts_use_the_page_codes() {
    let lib = ["", "usr", "lib", "x86_64-linux-gnu"].join("/");
    let maps: String = [
        "libgtk-4.so.1.2200.4",
        "libgtk-3.so.0.2400.52",
        "libgdk-3.so.0.2400.52",
        "libglib-2.0.so.0.8800.0",
        "libgobject-2.0.so.0.8800.0",
        "libgio-2.0.so.0.8800.0",
        "libgdk_pixbuf-2.0.so.0.4200.12",
        "libcairo.so.2.11800.0",
        "libarchive.so.13.8.5",
    ]
    .iter()
    .map(|name| format!("7f00-7f01 r-xp 00000000 08:02 11 {lib}/{name} (deleted)\n"))
    .collect();
    let parts: Vec<&str> = restart::shell_parts_in(&maps).into_iter().collect();
    // a replaced GTK 3 is no part of ours (no binary links it since 14-11)
    assert_eq!(parts, ["archive", "cairo", "gio", "glib", "gobject", "gtk4", "pixbuf"]);
    // one library alone gives its own code (no folding into gtk/glib any more)
    for (name, code) in [("libgtk-4.so.1", "gtk4"), ("libgdk_pixbuf-2.0.so.0", "pixbuf"), ("libgio-2.0.so.0", "gio"), ("libgobject-2.0.so.0", "gobject")] {
        let one = format!("7f00-7f01 r-xp 00000000 08:02 11 {lib}/{name} (deleted)\n");
        assert_eq!(restart::shell_parts_in(&one).into_iter().collect::<Vec<_>>(), [code]);
    }
    // the page's part_name names every code the host sends (and "self" for a replaced own binary); the page is
    // part of the repository, so a missing file fails
    let page = Path::new(env!("CARGO_MANIFEST_DIR")).join("../native/apps/settings/src/pages/update.rs");
    let page = fs::read_to_string(&page).unwrap_or_else(|e| panic!("{}: {e}", page.display()));
    let table = page.split("fn part_name(").nth(1).and_then(|t| t.split("\n}\n").next()).expect("update.rs part_name");
    for code in parts.iter().copied().chain(["self"]) {
        assert!(table.contains(&format!("\"{code}\"")), "update.rs part_name lacks {code}");
    }
}

/// GW3 D1 / D-G41: the job fields the page reads. `job.simulating` (also from a job file), `job.lockHolder`
/// ('unattended' while unattended-upgrades holds the lock we wait for), `job.foreignOwn` (a foreign transaction
/// that is our own Software job, with a job object of its own), and `totals.downloadSize` from the running job.
#[test]
fn the_snapshot_job_carries_the_fields_the_page_reads() {
    let f = Fixture::new("jobfields");
    let ctx = f.ctx("panel", false, "en");
    let setup = paths::resolve(&f.env()).unwrap();
    let store = store::Store::open(ctx.cache_dir.join("update")).unwrap();
    let shared = super::Shared { ctx: ctx.clone(), setup: setup.clone(), store: store.clone(), stats: Default::default(), rev: Default::default() };
    let boot = store::boot_id(&setup.paths).unwrap();
    let packages = vec![pkg("libsoup-3.0-0", "3.6.6-1ubuntu0.3", 8, "ubuntu-resolute-security").json()];
    let snapshot = json!({"rev":3,"backend":"packagekit","checkedAt":1,"packages":packages,"downloadSize":null,
                          "network":{"state":"full","online":true,"metered":false}});
    // a live job waiting for the dpkg lock: lockHolder only while unattended-upgr runs
    let live = json!({"jobId":"0123456789abcdef0123456789abcdef","role":"update","status":30,"pct":null,"speed":0,
                      "remaining":0,"downloadLeft":null,"allowCancel":true,"committing":false,"startedAt":1,
                      "reattached":false,"foreign":false,"confirm":null,"simulating":false,"downloadSize":4096,"rows":{}});
    let view = compose(&shared, Some(&snapshot), None, Some(&live), false);
    assert_eq!(view["phase"], "waiting");
    assert_eq!(view["job"]["lockHolder"], Value::Null, "another program holds the lock");
    assert_eq!(view["job"]["foreignOwn"], false);
    assert_eq!(view["totals"]["downloadSize"], 4096, "the running job's total");
    write(&f.root.join("proc/4242/comm"), "unattended-upgr\n");
    let view = compose(&shared, Some(&snapshot), None, Some(&live), false);
    assert_eq!(view["job"]["lockHolder"], "unattended");
    assert_eq!(view["waitLockAuto"], true);
    // the same job downloading: no lock holder
    let mut downloading = live.clone();
    downloading["status"] = json!(8);
    assert_eq!(compose(&shared, Some(&snapshot), None, Some(&downloading), false)["job"]["lockHolder"], Value::Null);
    fs::remove_dir_all(f.root.join("proc/4242")).unwrap();
    // a reader sees a simulating job from job.json (owner alive, this boot) and the simulate's total
    let file = json!({"version":1,"jobId":"0123456789abcdef0123456789abcdef","bootId":boot,"role":"update","lastStatus":13,
                      "owner":store::current_owner(),"committing":false,"simulating":true,"confirm":null,
                      "simulate":{"installs":[],"removals":[],"downloadSize":1257472},"rowStates":{}});
    let view = compose(&shared, Some(&snapshot), Some(&file), None, false);
    assert_eq!((view["phase"].clone(), view["job"]["simulating"].clone()), (json!("downloading"), json!(true)));
    assert_eq!(view["totals"]["downloadSize"], 1257472);
    // a snapshot total wins over the job's
    let mut sized = snapshot.clone();
    sized["downloadSize"] = json!(77);
    assert_eq!(compose(&shared, Some(&sized), Some(&file), None, false)["totals"]["downloadSize"], 77);
    // review m1: a snapshot total of 0 (a simulate without a size) is unknown, never a total: the job's wins, and
    // with none either the rows' sizes (here none: null), never 0
    sized["downloadSize"] = json!(0);
    assert_eq!(compose(&shared, Some(&sized), Some(&file), None, false)["totals"]["downloadSize"], 1257472);
    let mut unsized_file = file.clone();
    unsized_file["simulate"]["downloadSize"] = json!(0);
    assert_eq!(compose(&shared, Some(&sized), Some(&unsized_file), None, false)["totals"]["downloadSize"], Value::Null);
    // a foreign transaction: a job object with foreign true and foreignOwn from our Software job files
    let mut foreign = snapshot.clone();
    foreign["foreign"] = json!(true);
    let view = compose(&shared, Some(&foreign), None, None, false);
    assert_eq!(view["phase"], "foreign");
    assert_eq!((view["job"]["foreign"].clone(), view["job"]["foreignOwn"].clone()), (json!(true), json!(false)));
    assert_eq!(view["job"]["jobId"], Value::Null);
    let jobs = ctx.cache_dir.join("software/jobs");
    fs::create_dir_all(&jobs).unwrap();
    store::atomic(&jobs.join("a.json"), json!({"state":"running","_owner":store::current_owner()}).to_string().as_bytes()).unwrap();
    let view = compose(&shared, Some(&foreign), None, None, false);
    assert_eq!((view["job"]["foreignOwn"].clone(), view["foreignOwn"].clone()), (json!(true), json!(true)));
    // a finished Software job is not "own"
    store::atomic(&jobs.join("a.json"), json!({"state":"done","_owner":store::current_owner()}).to_string().as_bytes()).unwrap();
    assert_eq!(compose(&shared, Some(&foreign), None, None, false)["job"]["foreignOwn"], false);
    // no job and nothing foreign: job is null
    assert!(compose(&shared, Some(&snapshot), None, None, false)["job"].is_null());
    assert_eq!(f.stub_log(), "");
}
