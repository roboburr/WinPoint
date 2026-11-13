//! Every system path and every program of the update service in one table, the test re-rooting, and the
//! fail-closed seam check (design §4.11).
//!
//! * Test mode is "any `WIN11_UPDATE_*` variable is set". Each variable must pass its check (euid ≠ 0, an
//!   absolute private directory with the marker file [`MARKER`], and for the bus contract C10: one `unix:path=`
//!   socket directly in a marked 0700 `$XDG_RUNTIME_DIR` outside `/run`, while the real system-bus socket is not
//!   visible); otherwise [`resolve`] fails and the service stays `unsupported` (`test_seam`) for the whole
//!   process life: no bus, no thread, no tool. There is never a fallback to the system bus or to `PATH`.
//! * In test mode every tool comes from `WIN11_UPDATE_TOOL_DIR` and nowhere else; a missing stub is a seam
//!   error at the call, never an absolute or `PATH` lookup.
//! * This file is the only file under `services/update/` that may hold an absolute exec path, and only between
//!   the `table:begin` / `table:end` lines (the grep test in `tests.rs` enforces it).
use std::{
    collections::BTreeMap,
    ffi::OsString,
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, PermissionsExt},
    },
    path::{Component, Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

/// The marker file a fixture, tool or bus directory must contain before a seam is honoured.
pub(crate) const MARKER: &str = ".win11-update-fixture";
pub(crate) const VAR_PREFIX: &str = "WIN11_UPDATE_";
pub(crate) const VAR_PK_BUS: &str = "WIN11_UPDATE_PK_BUS";
pub(crate) const VAR_FIXTURE_ROOT: &str = "WIN11_UPDATE_FIXTURE_ROOT";
pub(crate) const VAR_TOOL_DIR: &str = "WIN11_UPDATE_TOOL_DIR";
const KNOWN_VARS: [&str; 3] = [VAR_PK_BUS, VAR_FIXTURE_ROOT, VAR_TOOL_DIR];

/// Every program the update service (and GW5–GW7 inside `services/update/`) may start.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Tool {
    Loginctl,
    XdgTerminalExec,
    Pkexec,
    CheckNewRelease,
    AptGet,
    AptCache,
    AptConfig,
    DpkgQuery,
    Snap,
    Python3,
    Screen,
    Nice,
    Ionice,
    Sh,
    DoReleaseUpgrade,
    PromptHelper,
    AttachHelper,
}

// table:begin (the only lines under services/update/ that may name an absolute program or system library path)
const SYSTEM_TOOLS: &[(Tool, &str, &str)] = &[
    (Tool::Loginctl, "loginctl", "/usr/bin/loginctl"),
    (Tool::XdgTerminalExec, "xdg-terminal-exec", "/usr/bin/xdg-terminal-exec"),
    (Tool::Pkexec, "pkexec", "/usr/bin/pkexec"),
    (Tool::CheckNewRelease, "check-new-release", "/usr/lib/ubuntu-release-upgrader/check-new-release"),
    (Tool::AptGet, "apt-get", "/usr/bin/apt-get"),
    (Tool::AptCache, "apt-cache", "/usr/bin/apt-cache"),
    (Tool::AptConfig, "apt-config", "/usr/bin/apt-config"),
    (Tool::DpkgQuery, "dpkg-query", "/usr/bin/dpkg-query"),
    (Tool::Snap, "snap", "/usr/bin/snap"),
    (Tool::Python3, "python3", "/usr/bin/python3"),
    (Tool::Screen, "screen", "/usr/bin/screen"),
    (Tool::Nice, "nice", "/usr/bin/nice"),
    (Tool::Ionice, "ionice", "/usr/bin/ionice"),
    (Tool::Sh, "sh", "/bin/sh"),
    (Tool::DoReleaseUpgrade, "do-release-upgrade", "/usr/bin/do-release-upgrade"),
    (Tool::PromptHelper, "release-upgrades-helper", "/usr/libexec/win11-gnome/release-upgrades-helper"),
    (Tool::AttachHelper, "release-upgrade-attach", "/usr/libexec/win11-gnome/release-upgrade-attach"),
];
/// The soname directory (compat, post-upgrade check); re-rooted in test mode.
const SONAME_DIR: &str = "/usr/lib/x86_64-linux-gnu";
/// WebKitGTK's helper programs (a WebKit child of ours, design §4.8).
pub(crate) const WEBKIT_EXE_DIR: &str = "/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1/";
/// Deleted mappings in the running gnome-shell that need a reboot (design §4.8 `gnome` list).
pub(crate) const GNOME_MAPS: &[&str] = &[
    "/usr/bin/gnome-shell",
    "/usr/lib/gnome-shell/",
    "/usr/lib/x86_64-linux-gnu/gnome-shell/",
    "libmutter-",
    "/usr/lib/x86_64-linux-gnu/mutter-",
];
/// Deleted mappings in our own processes that need a Win11 restart (design §4.8 `shell` list), by the component
/// code the page reads in `restart.shellParts` (`update.js` PART: webkit, jsc, gtk, gdk, soup, glib, gio, gobject,
/// pixbuf, cairo, archive, and `self` for a replaced own binary; GW3 D1, D-G41).
pub(crate) const SHELL_MAPS: &[(&str, &str)] = &[
    ("libwebkit2gtk-4.1.so", "webkit"),
    ("libjavascriptcoregtk-4.1.so", "jsc"),
    ("/webkit2gtk-4.1/", "webkit"),
    ("libgtk-3.so", "gtk"),
    ("libgdk-3.so", "gdk"),
    ("libsoup-3.0.so", "soup"),
    ("libglib-2.0.so", "glib"),
    ("libgobject-2.0.so", "gobject"),
    ("libgio-2.0.so", "gio"),
    ("libgdk_pixbuf-2.0.so", "pixbuf"),
    ("libcairo.so", "cairo"),
    ("libarchive.so", "archive"),
];
/// The real system-bus socket: while it is visible, a test bus is refused (contract C10). Not an exec path.
const SYSTEM_BUS_SOCKETS: [&str; 2] = ["/run/dbus/system_bus_socket", "/var/run/dbus/system_bus_socket"];
/// `PATH` of every production child (design §4.11, review F1): nothing the user's environment adds.
const SYSTEM_PATH: &str = "/usr/bin:/bin";
// table:end

impl Tool {
    #[cfg_attr(not(test), allow(dead_code))] // the tests' stub directory
    pub(crate) const ALL: [Tool; 17] = [
        Tool::Loginctl,
        Tool::XdgTerminalExec,
        Tool::Pkexec,
        Tool::CheckNewRelease,
        Tool::AptGet,
        Tool::AptCache,
        Tool::AptConfig,
        Tool::DpkgQuery,
        Tool::Snap,
        Tool::Python3,
        Tool::Screen,
        Tool::Nice,
        Tool::Ionice,
        Tool::Sh,
        Tool::DoReleaseUpgrade,
        Tool::PromptHelper,
        Tool::AttachHelper,
    ];
    fn entry(self) -> (&'static str, &'static str) {
        SYSTEM_TOOLS
            .iter()
            .find(|(tool, _, _)| *tool == self)
            .map(|(_, name, path)| (*name, *path))
            .unwrap_or(("", ""))
    }
    /// The stub's file name in `WIN11_UPDATE_TOOL_DIR`.
    pub(crate) fn name(self) -> &'static str {
        self.entry().0
    }
}

/// Why a seam (or, at a call, a tool) was refused. Never shown as is: the service answers `update.err.testSeam`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SeamError {
    pub variable: String,
    pub reason: &'static str,
}
impl SeamError {
    fn new(variable: &str, reason: &'static str) -> Self {
        Self { variable: variable.into(), reason }
    }
}

/// Where PackageKit, NetworkManager and logind calls go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BusTarget {
    /// the real system bus (no test seam set)
    System,
    /// a private test bus (`WIN11_UPDATE_PK_BUS`, checked)
    Address(String),
    /// test mode without a bus: nothing is ever connected
    Disabled,
}

/// The `WIN11_UPDATE_*` part of the environment, the effective uid, `$XDG_RUNTIME_DIR` and the real system-bus
/// socket paths (injected by unit tests; contract C10 needs the last two).
#[derive(Clone, Debug, Default)]
pub(crate) struct Env {
    vars: BTreeMap<Vec<u8>, OsString>,
    euid: u32,
    /// `$XDG_RUNTIME_DIR`: a test bus socket must lie directly in it (C10)
    runtime_dir: Option<OsString>,
    /// the real system-bus socket paths: a test bus is refused while any of them is visible (C10)
    system_sockets: Vec<PathBuf>,
}
impl Env {
    /// This process's environment.
    pub(crate) fn process() -> Self {
        let vars = std::env::vars_os()
            .filter(|(key, _)| key.as_bytes().starts_with(VAR_PREFIX.as_bytes()))
            .map(|(key, value)| (key.into_vec(), value))
            .collect();
        Self {
            vars,
            euid: unsafe { libc::geteuid() },
            runtime_dir: std::env::var_os("XDG_RUNTIME_DIR"),
            system_sockets: SYSTEM_BUS_SOCKETS.iter().map(PathBuf::from).collect(),
        }
    }
    /// Unit tests: these variables and this euid, no runtime directory, and the real system-bus socket paths (so a
    /// test bus fails closed unless the test sets both with [`Env::runtime`] and [`Env::system_sockets`]).
    #[cfg(test)]
    pub(crate) fn fixture(pairs: &[(&str, &str)], euid: u32) -> Self {
        Self {
            vars: pairs
                .iter()
                .map(|(key, value)| (key.as_bytes().to_vec(), OsString::from(value)))
                .collect(),
            euid,
            runtime_dir: None,
            system_sockets: SYSTEM_BUS_SOCKETS.iter().map(PathBuf::from).collect(),
        }
    }
    #[cfg(test)]
    pub(crate) fn runtime(mut self, dir: Option<&Path>) -> Self {
        self.runtime_dir = dir.map(|d| d.as_os_str().to_owned());
        self
    }
    #[cfg(test)]
    pub(crate) fn system_sockets(mut self, paths: Vec<PathBuf>) -> Self {
        self.system_sockets = paths;
        self
    }
    pub(crate) fn test_mode(&self) -> bool {
        !self.vars.is_empty()
    }
    fn get(&self, key: &str) -> Option<&OsString> {
        self.vars.get(key.as_bytes())
    }
}

/// Every fixed file the service reads (re-rooted under `WIN11_UPDATE_FIXTURE_ROOT` in test mode). Fields that
/// GW2 does not read yet belong to GW4–GW7 (history, release, compat, upgrade, post-upgrade).
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct Paths {
    /// the procfs used for scans (`/proc`, or `<fixture>/proc`): maps, cmdline, comm, locks, boot id
    pub proc: PathBuf,
    pub boot_id: PathBuf,
    pub proc_locks: PathBuf,
    pub reboot_required: PathBuf,
    pub reboot_required_pkgs: PathBuf,
    pub update_manager_etc: PathBuf,
    pub update_manager_var: PathBuf,
    pub release_upgrader_var: PathBuf,
    pub distro_info: PathBuf,
    pub apt_log: PathBuf,
    pub dist_upgrade_log: PathBuf,
    pub os_release: PathBuf,
    pub apt_sources_d: PathBuf,
    pub archive_keyring: PathBuf,
    pub dpkg_lock_frontend: PathBuf,
    pub dpkg_lock: PathBuf,
    pub apt_lists_lock: PathBuf,
    pub apt_archives_lock: PathBuf,
    pub dpkg_status: PathBuf,
    pub apt_extended_states: PathBuf,
    pub soname_dir: PathBuf,
    /// true in test mode with a fixture root (scans then also accept fixture uids and exe links)
    pub fixture: bool,
    // GW4: update-notifier's GSettings (design §4.12 D2). None in production (the default schema source and the
    // default backend, dconf); with a fixture root the compiled schemas `<root>/usr/share/glib-2.0/schemas` and a
    // keyfile backend `<root>/home/.config/glib-2.0/settings/keyfile`, so a test never reaches the real dconf.
    pub gsettings_schemas: Option<PathBuf>,
    pub gsettings_keyfile: Option<PathBuf>,
    // GW5: the user's cache directory ($XDG_CACHE_HOME or ~/.cache: update-manager-core's own meta-release copies,
    // and check-new-release's XDG_CACHE_HOME) and home; `<root>/home/.cache` and `<root>/home` with a fixture root.
    pub user_cache: PathBuf,
    pub user_home: PathBuf,
    // GW6: the compatibility check's inputs (design §6.2). `user_data` ($XDG_DATA_HOME, where the installed extension's
    // gnome-shell/extensions/<uuid>/metadata.json lives); `bin_dir` (our win11-* binaries for the NEEDED scan: the
    // running program's directory, " (deleted)" stripped); `releases` (None: the embedded host/data/releases.json;
    // `<root>/releases.json` with a fixture root, read when it exists); `locale_dirs` (ubuntu-release-upgrader's Dutch
    // catalogue, the start dialog's relEnglishFirst). With a fixture root: `<root>/home/.local/share`, `<root>/bin` and
    // the re-rooted locale directories. The private apt tree itself lives in the service's cache directory
    // (`<cache>/update/release-check/<codename>/`); its system inputs are `apt_sources_d`, `archive_keyring` (GW6's
    // throwaway key in test mode), `dpkg_status` and `apt_extended_states` above, all re-rooted with the fixture.
    pub user_data: PathBuf,
    pub bin_dir: Option<PathBuf>,
    pub releases: Option<PathBuf>,
    pub locale_dirs: Vec<PathBuf>,
}
impl Paths {
    fn rooted(root: Option<&Path>) -> Self {
        let at = |system: &str| match root {
            Some(root) => root.join(system.trim_start_matches('/')),
            None => PathBuf::from(system),
        };
        let proc = at("/proc");
        Self {
            boot_id: proc.join("sys/kernel/random/boot_id"),
            proc_locks: proc.join("locks"),
            proc,
            reboot_required: at("/run/reboot-required"),
            reboot_required_pkgs: at("/run/reboot-required.pkgs"),
            update_manager_etc: at("/etc/update-manager"),
            update_manager_var: at("/var/lib/update-manager"),
            release_upgrader_var: at("/var/lib/ubuntu-release-upgrader"),
            distro_info: at("/usr/share/distro-info/ubuntu.csv"),
            apt_log: at("/var/log/apt"),
            dist_upgrade_log: at("/var/log/dist-upgrade"),
            os_release: at("/etc/os-release"),
            apt_sources_d: at("/etc/apt/sources.list.d"),
            archive_keyring: at("/usr/share/keyrings/ubuntu-archive-keyring.gpg"),
            dpkg_lock_frontend: at("/var/lib/dpkg/lock-frontend"),
            dpkg_lock: at("/var/lib/dpkg/lock"),
            apt_lists_lock: at("/var/lib/apt/lists/lock"),
            apt_archives_lock: at("/var/cache/apt/archives/lock"),
            dpkg_status: at("/var/lib/dpkg/status"),
            apt_extended_states: at("/var/lib/apt/extended_states"),
            soname_dir: at(SONAME_DIR),
            fixture: root.is_some(),
            // GW4
            gsettings_schemas: root.map(|root| root.join("usr/share/glib-2.0/schemas")),
            gsettings_keyfile: root.map(|root| root.join("home/.config/glib-2.0/settings/keyfile")),
            // GW5
            user_cache: root.map_or_else(glib::user_cache_dir, |root| root.join("home/.cache")),
            user_home: root.map_or_else(glib::home_dir, |root| root.join("home")),
            // GW6
            user_data: root.map_or_else(glib::user_data_dir, |root| root.join("home/.local/share")),
            bin_dir: match root {
                Some(root) => Some(root.join("bin")),
                None => std::env::current_exe().ok().and_then(|exe| {
                    let text = exe.to_string_lossy();
                    Path::new(text.strip_suffix(" (deleted)").unwrap_or(&text)).parent().map(Path::to_path_buf)
                }),
            },
            releases: root.map(|root| root.join("releases.json")),
            locale_dirs: vec![at("/usr/share/locale"), at("/usr/share/locale-langpack")],
        }
    }
    /// The four dpkg/apt lock files of the `/proc/locks` busy rule (design §6.4).
    pub(crate) fn apt_locks(&self) -> [&Path; 4] {
        [&self.dpkg_lock_frontend, &self.dpkg_lock, &self.apt_lists_lock, &self.apt_archives_lock]
    }
}

/// The program table: the system paths, or only the stub directory in test mode. Counts every start.
#[derive(Clone, Debug)]
pub(crate) struct Tools {
    stubs: Option<PathBuf>,
    test: bool,
    runs: Arc<AtomicUsize>,
    /// The environment of every child besides `LANG` (review F1). Production: `PATH=/usr/bin:/bin`. Test mode:
    /// the stub directory as the whole `PATH`, the checked private bus (or a dead address in the stub
    /// directory) as `DBUS_SYSTEM_BUS_ADDRESS`, and a dead session bus, so a stub's own children and bus clients
    /// can never fall back to `/usr/bin` or to the real buses.
    env: Vec<(&'static str, String)>,
}
/// The dead bus addresses of test mode: sockets that never exist inside the marked stub directory.
pub(crate) const NO_SYSTEM_BUS: &str = "no-system-bus";
pub(crate) const NO_SESSION_BUS: &str = "no-session-bus";
impl Tools {
    fn system() -> Self {
        Self { stubs: None, test: false, runs: Arc::default(), env: vec![("PATH", SYSTEM_PATH.to_owned())] }
    }
    /// Test mode: `dir` (checked) or no tools at all.
    fn stubs(dir: Option<PathBuf>, bus: &BusTarget) -> Self {
        let env = match &dir {
            Some(dir) => {
                let dead = |name: &str| format!("unix:path={}", dir.join(name).display());
                let system = match bus {
                    BusTarget::Address(address) => address.clone(),
                    BusTarget::System | BusTarget::Disabled => dead(NO_SYSTEM_BUS),
                };
                vec![
                    ("PATH", dir.display().to_string()),
                    ("DBUS_SYSTEM_BUS_ADDRESS", system),
                    ("DBUS_SESSION_BUS_ADDRESS", dead(NO_SESSION_BUS)),
                ]
            }
            // no tool directory: no program can start at all (`path` fails)
            None => Vec::new(),
        };
        Self { stubs: dir, test: true, runs: Arc::default(), env }
    }
    /// The program to run for `tool`. In test mode only an executable stub inside the tool directory.
    pub(crate) fn path(&self, tool: Tool) -> Result<PathBuf, SeamError> {
        if !self.test {
            return Ok(PathBuf::from(tool.entry().1));
        }
        let Some(dir) = &self.stubs else {
            return Err(SeamError::new(VAR_TOOL_DIR, "test mode without a tool directory"));
        };
        let stub = dir.join(tool.name());
        let inside = stub
            .canonicalize()
            .ok()
            .zip(dir.canonicalize().ok())
            .is_some_and(|(real, dir)| real.starts_with(dir));
        let executable = std::fs::metadata(&stub)
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
        if inside && executable {
            Ok(stub)
        } else {
            Err(SeamError::new(VAR_TOOL_DIR, "stub missing"))
        }
    }
    /// A `Command` for `tool` with a cleared environment plus `LANG=C.UTF-8` and the table's `PATH` and bus
    /// addresses (callers add only what they need). Counts the start.
    pub(crate) fn command(&self, tool: Tool) -> Result<Command, SeamError> {
        let path = self.path(tool)?;
        self.runs.fetch_add(1, Ordering::SeqCst);
        let mut command = Command::new(path);
        command.env_clear();
        command.env("LANG", "C.UTF-8");
        for (key, value) in &self.env {
            command.env(key, value);
        }
        Ok(command)
    }
    /// How many programs this table started (tests: dry-run and seam failures start none).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn runs(&self) -> usize {
        self.runs.load(Ordering::SeqCst)
    }
}

/// The result of the seam check: where the bus is, which files and which programs.
#[derive(Clone, Debug)]
pub(crate) struct Setup {
    #[allow(dead_code)] // read by GW5–GW7 (e.g. never a real gsettings write in test mode)
    pub test: bool,
    pub bus: BusTarget,
    pub paths: Paths,
    pub tools: Tools,
}

/// The seam check (design §4.11). Without any `WIN11_UPDATE_*` variable: the system. With one: every variable
/// must pass, or the whole setup fails (fail closed).
pub(crate) fn resolve(env: &Env) -> Result<Setup, SeamError> {
    if !env.test_mode() {
        return Ok(Setup {
            test: false,
            bus: BusTarget::System,
            paths: Paths::rooted(None),
            tools: Tools::system(),
        });
    }
    for key in env.vars.keys() {
        let name = String::from_utf8_lossy(key);
        if !KNOWN_VARS.contains(&name.as_ref()) {
            return Err(SeamError::new(&name, "unknown variable"));
        }
    }
    if env.euid == 0 {
        return Err(SeamError::new(VAR_PREFIX, "euid 0"));
    }
    let text = |key: &str| -> Result<Option<String>, SeamError> {
        match env.get(key) {
            None => Ok(None),
            Some(value) => value
                .to_str()
                .filter(|v| !v.is_empty())
                .map(|v| Some(v.to_owned()))
                .ok_or_else(|| SeamError::new(key, "empty or not UTF-8")),
        }
    };
    let root = match text(VAR_FIXTURE_ROOT)? {
        Some(dir) => Some(check_dir(VAR_FIXTURE_ROOT, Path::new(&dir), env.euid)?),
        None => None,
    };
    let tools = match text(VAR_TOOL_DIR)? {
        Some(dir) => Some(check_dir(VAR_TOOL_DIR, Path::new(&dir), env.euid)?),
        None => None,
    };
    let bus = match text(VAR_PK_BUS)? {
        Some(address) => BusTarget::Address(check_bus(&address, env)?),
        None => BusTarget::Disabled,
    };
    let tools = Tools::stubs(tools, &bus);
    Ok(Setup {
        test: true,
        bus,
        paths: Paths::rooted(root.as_deref()),
        tools,
    })
}

/// An absolute, normal (no `..`), non-symlink directory of ours, not group/world writable, not `/`, with the
/// marker file in it.
fn check_dir(variable: &str, dir: &Path, euid: u32) -> Result<PathBuf, SeamError> {
    if !dir.is_absolute() || dir.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return Err(SeamError::new(variable, "not an absolute normal path"));
    }
    if dir.parent().is_none() {
        return Err(SeamError::new(variable, "the root directory"));
    }
    let meta = std::fs::symlink_metadata(dir).map_err(|_| SeamError::new(variable, "missing directory"))?;
    if !meta.is_dir() || meta.uid() != euid || meta.mode() & 0o022 != 0 {
        return Err(SeamError::new(variable, "not a private directory of this user"));
    }
    let marker = std::fs::symlink_metadata(dir.join(MARKER));
    if !marker.is_ok_and(|m| m.is_file() && m.uid() == euid) {
        return Err(SeamError::new(variable, "marker missing"));
    }
    Ok(dir.to_path_buf())
}

/// `/run`, or a path below it (`/var/run` is its link).
/// Present unless it is certainly absent: any error but NotFound (EACCES on a closed directory, ELOOP, ENOTDIR)
/// counts as present, so the system-bus check fails closed where the path cannot be inspected (review n4).
pub(crate) fn maybe_present(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(_) => true,
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}
fn under_run(path: &Path) -> bool {
    path.starts_with("/run") || path.starts_with("/var/run")
}
/// An absolute path without `.` or `..` components.
fn normal_absolute(path: &Path) -> bool {
    path.is_absolute() && !path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir))
}
/// The test-bus rule, contract C10 (D-G39), exactly:
/// * the value is one `unix:path=` address with at most one `,guid=<32 hex>` (no abstract, tcp, escapes or second
///   address);
/// * the socket lies directly in `$XDG_RUNTIME_DIR`, which is not under `/run` (also not through a symlink);
/// * that directory is owned by the euid, has mode 0700 and holds the [`MARKER`]; the socket is a socket of ours;
/// * the real `/run/dbus/system_bus_socket` is not visible (the caller runs inside a layer with a private `/run`).
///
/// Anything else is a seam error: the service then opens no bus at all (fail closed).
fn check_bus(address: &str, env: &Env) -> Result<String, SeamError> {
    let refuse = |reason| Err(SeamError::new(VAR_PK_BUS, reason));
    if address.contains(';') {
        return refuse("more than one address");
    }
    let Some(rest) = address.strip_prefix("unix:path=") else {
        return refuse("not a unix:path= address");
    };
    let mut parts = rest.split(',');
    let path = parts.next().unwrap_or("");
    let mut guid = false;
    for part in parts {
        match part.strip_prefix("guid=") {
            Some(g) if !guid && g.len() == 32 && g.bytes().all(|b| b.is_ascii_hexdigit()) => guid = true,
            _ => return refuse("a key other than one guid="),
        }
    }
    if path.is_empty() || path.contains(['%', '=']) {
        return refuse("not a plain socket path");
    }
    let socket = Path::new(path);
    if !normal_absolute(socket) {
        return refuse("not an absolute normal path");
    }
    if under_run(socket) {
        return refuse("the socket is under /run");
    }
    if env.system_sockets.iter().any(|p| maybe_present(p)) {
        return refuse("the real system bus socket is visible");
    }
    let Some(runtime) = env.runtime_dir.as_ref().filter(|d| !d.is_empty()).map(PathBuf::from) else {
        return refuse("XDG_RUNTIME_DIR is not set");
    };
    if !normal_absolute(&runtime) {
        return refuse("XDG_RUNTIME_DIR is not an absolute normal path");
    }
    if under_run(&runtime) {
        return refuse("XDG_RUNTIME_DIR is under /run");
    }
    if socket.parent() != Some(runtime.as_path()) {
        return refuse("the socket is not directly in XDG_RUNTIME_DIR");
    }
    // a symlinked component must not lead under /run either
    match std::fs::canonicalize(&runtime) {
        Ok(real) if under_run(&real) => return refuse("XDG_RUNTIME_DIR is under /run"),
        Ok(_) => {}
        Err(_) => return refuse("XDG_RUNTIME_DIR is missing"),
    }
    let meta = std::fs::symlink_metadata(&runtime).map_err(|_| SeamError::new(VAR_PK_BUS, "XDG_RUNTIME_DIR is missing"))?;
    if !meta.is_dir() || meta.uid() != env.euid || meta.mode() & 0o777 != 0o700 {
        return refuse("XDG_RUNTIME_DIR is not a 0700 directory of this user");
    }
    if !std::fs::symlink_metadata(runtime.join(MARKER)).is_ok_and(|m| m.is_file() && m.uid() == env.euid) {
        return refuse("marker missing");
    }
    use std::os::unix::fs::FileTypeExt;
    let meta = std::fs::symlink_metadata(socket).map_err(|_| SeamError::new(VAR_PK_BUS, "no socket"))?;
    if !meta.file_type().is_socket() || meta.uid() != env.euid {
        return refuse("not a socket of this user");
    }
    Ok(address.to_owned())
}

/// The seam setup of this process (the guard and the service resolve it the same way).
pub(crate) fn process_setup() -> Result<Setup, SeamError> {
    resolve(&Env::process())
}
