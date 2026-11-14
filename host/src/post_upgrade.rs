//! `win11-launch --post-upgrade-check` (design §6.6, GW7): the XDG autostart check outside the extension, so it
//! runs when a library of our programs is gone or GNOME rejected our extension. GIO only (win11-launch's NEEDED
//! stays as it is).
//!
//! 1. `/etc/os-release` (`VERSION_ID`, `VERSION_CODENAME`, `VERSION`), the GNOME major from the `org.gnome.Shell`
//!    `ShellVersion` property (≤ 30 s of retries) or `dpkg-query`, and `~/.config/win11-gnome/last-os-release.json`.
//!    No record: write it (and the linger cleanup of step 5) and stop. Nothing changed: only step 5, when our linger
//!    stamp exists. While Ubuntu's upgrader runs nothing happens at all (the next login after its reboot checks).
//! 2. On a change: `org.gnome.shell disable-user-extensions` true → `update.post.extsOff` (actions extsOn, dockOn).
//!    Otherwise `GetExtensionInfo(win11-gnome@win11shell.local)`'s `state`, read as a double and rounded (a missing or
//!    non-numeric one counts as ERROR): 1 → `update.post.upgraded`; 4 OUT_OF_DATE, or 2 INACTIVE while our uuid is in
//!    `enabled-extensions` → `update.post.extOff`; 3 ERROR → `update.post.extError` (both: dockOn, later).
//! 3. A blocking soname missing on disk (the design §6.2 list) → `update.post.missing` (installParts, later).
//! 4. GNOME's `org.freedesktop.Notifications`; ActionInvoked is awaited ≤ 10 min. `dockOn` →
//!    `EnableExtension("ubuntu-dock@ubuntu.com")`, `extsOn` → `disable-user-extensions = false`, `installParts` → the
//!    terminal wrapper (hold always) running `pkexec apt-get install <names>`, the names resolved on the new release
//!    (`apt-cache policy`, else the single "Reverse Provides:" package of `apt-cache showpkg`, else the old name),
//!    started through `process_scope::spawn` (its own unit in `app.slice`, not the autostart unit). Only on that click.
//! 5. Our linger stamp and no upgrader → `loginctl disable-linger <uid>`, the stamp goes (done before step 4 waits, so a
//!    logout during the wait cannot leave linger on).
//! 6. `last-os-release.json` is written; `compat-*.json`, `release.json`, `upgrade.json` and the wrapper's status files
//!    go.
//!
//! Language: `settings.json` (C1, default nl) through the bin's own `i18n.rs` (the rows of `host/i18n/update.tsv`,
//! no lib crate). Programs: only this file's tool table; in test mode (any `WIN11_UPDATE_*` variable) only the stubs
//! of `WIN11_UPDATE_TOOL_DIR`, and every system path under `WIN11_UPDATE_FIXTURE_ROOT` (required); a seam that
//! fails its check stops everything (fail closed).
use gio::prelude::*;
use glib::variant::ToVariant;
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    ffi::OsString,
    fs,
    io::{Read, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    rc::Rc,
    time::{Duration, Instant},
};

pub(crate) const FLAG: &str = "--post-upgrade-check";
const UUID: &str = "win11-gnome@win11shell.local";
const DOCK: &str = "ubuntu-dock@ubuntu.com";
const RECORD: &str = "last-os-release.json";
const LINGER_STAMP: &str = "update-linger-by-us";
const MARKER: &str = ".win11-update-fixture";
const VAR_PREFIX: &str = "WIN11_UPDATE_";
const KNOWN_VARS: [&str; 3] = [
    "WIN11_UPDATE_PK_BUS",
    "WIN11_UPDATE_FIXTURE_ROOT",
    "WIN11_UPDATE_TOOL_DIR",
];
const SHELL_SCHEMA: &str = "org.gnome.shell";
const KEY_DISABLE: &str = "disable-user-extensions";
const KEY_ENABLED: &str = "enabled-extensions";
const PARTS_NAME: &str = "win11-gnome-parts";
const PARTS_STATUS: &str = "parts-status.json";
const UPGRADE_SCREEN: &str = "ubuntu-release-upgrade-screen-window";
const NO_SYSTEM_BUS: &str = "no-system-bus";
const NO_SESSION_BUS: &str = "no-session-bus";

/// The terminal wrapper of `services/update/upgrade.rs` (design §6.4 step 4), the same text (a lib test compares).
const WRAPPER: &str = r#"status=$1; hold=$2; shift 2
"$@"; rc=$?
printf '{"rc":%d,"endedAt":%s}\n' "$rc" "$(date +%s)" > "$status.tmp" && mv -f "$status.tmp" "$status"
if [ "$hold" = always ] || [ "$rc" -ne 0 ]; then printf '\n%s\n' "$WIN11_PRESS_ENTER"; read -r _; fi
exit "$rc""#;

/// Every program this check may start (design §4.11: its own table, win11-launch does not link the lib crate).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tool {
    Loginctl,
    XdgTerminalExec,
    Sh,
    Pkexec,
    AptGet,
    AptCache,
    DpkgQuery,
}

// table:begin (the only lines of this file that may name an absolute program or system path)
const TOOLS: &[(Tool, &str, &str)] = &[
    (Tool::Loginctl, "loginctl", "/usr/bin/loginctl"),
    (
        Tool::XdgTerminalExec,
        "xdg-terminal-exec",
        "/usr/bin/xdg-terminal-exec",
    ),
    (Tool::Sh, "sh", "/bin/sh"),
    (Tool::Pkexec, "pkexec", "/usr/bin/pkexec"),
    (Tool::AptGet, "apt-get", "/usr/bin/apt-get"),
    (Tool::AptCache, "apt-cache", "/usr/bin/apt-cache"),
    (Tool::DpkgQuery, "dpkg-query", "/usr/bin/dpkg-query"),
];
const OS_RELEASE: &str = "/etc/os-release";
const PROC: &str = "/proc";
const SONAME_DIR: &str = "/usr/lib/x86_64-linux-gnu";
const SYSTEM_PATH: &str = "/usr/bin:/bin";
// table:end

/// The blocking libraries of design §6.2 (the NEEDED sonames of the win11-* binaries of scripts/install.py `BINS`
/// plus libarchive, loaded at run time; libc, libm, libgcc_s and ld-linux are not counted): soname, its 26.04
/// package, the part's label row. Graphene and Pango come with GTK 4 (the native shell and apps), so they carry its
/// label.
const PARTS: &[(&str, &str, &str)] = &[
    ("libgtk-4.so.1", "libgtk-4-1", "update.part.gtk4"),
    (
        "libgraphene-1.0.so.0",
        "libgraphene-1.0-0",
        "update.part.gtk4",
    ),
    ("libpango-1.0.so.0", "libpango-1.0-0", "update.part.gtk4"),
    ("libglib-2.0.so.0", "libglib2.0-0t64", "update.part.glib"),
    ("libgobject-2.0.so.0", "libglib2.0-0t64", "update.part.glib"),
    ("libgio-2.0.so.0", "libglib2.0-0t64", "update.part.glib"),
    (
        "libgdk_pixbuf-2.0.so.0",
        "libgdk-pixbuf-2.0-0",
        "update.part.pixbuf",
    ),
    ("libcairo.so.2", "libcairo2", "update.part.cairo"),
    ("libz.so.1", "zlib1g", "update.part.zlib"),
    ("libarchive.so.13", "libarchive13t64", "update.part.archive"),
];

// ------------------------------------------------------------------------------------------------ seams

/// Production, or test mode with its checked fixture root and tool directory.
#[derive(Clone, Debug, Default)]
struct Setup {
    test: bool,
    root: Option<PathBuf>,
    tools: Option<PathBuf>,
}
/// An absolute, normal, non-symlink directory of ours, not group/world writable, not `/`, holding the marker (the
/// rule of `services/update/paths.rs`).
fn check_dir(dir: &Path, euid: u32) -> Result<PathBuf, &'static str> {
    if !dir.is_absolute()
        || dir
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        || dir.parent().is_none()
    {
        return Err("not an absolute normal path");
    }
    let meta = fs::symlink_metadata(dir).map_err(|_| "missing directory")?;
    if !meta.is_dir() || meta.uid() != euid || meta.mode() & 0o022 != 0 {
        return Err("not a private directory of this user");
    }
    if !fs::symlink_metadata(dir.join(MARKER)).is_ok_and(|m| m.is_file() && m.uid() == euid) {
        return Err("marker missing");
    }
    Ok(dir.to_path_buf())
}
/// The seam check (design §4.11): no `WIN11_UPDATE_*` variable → production; any → every one must be known and pass,
/// the fixture root is required, and euid 0 is refused.
fn setup_from(vars: &[(String, OsString)], euid: u32) -> Result<Setup, &'static str> {
    if vars.is_empty() {
        return Ok(Setup::default());
    }
    if vars.iter().any(|(k, _)| !KNOWN_VARS.contains(&k.as_str())) {
        return Err("unknown variable");
    }
    if euid == 0 {
        return Err("euid 0");
    }
    let get = |name: &str| {
        vars.iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| PathBuf::from(v))
    };
    let root = check_dir(
        &get("WIN11_UPDATE_FIXTURE_ROOT").ok_or("no fixture root")?,
        euid,
    )?;
    let tools = match get("WIN11_UPDATE_TOOL_DIR") {
        Some(dir) => Some(check_dir(&dir, euid)?),
        None => None,
    };
    Ok(Setup {
        test: true,
        root: Some(root),
        tools,
    })
}
impl Setup {
    fn at(&self, system: &str) -> PathBuf {
        match &self.root {
            Some(root) => root.join(system.trim_start_matches('/')),
            None => PathBuf::from(system),
        }
    }
    fn home(&self) -> PathBuf {
        self.root
            .as_ref()
            .map_or_else(glib::home_dir, |root| root.join("home"))
    }
    /// `$XDG_RUNTIME_DIR/win11-gnome` (the fixture home's `.run/win11-gnome`, as upgrade.rs).
    fn runtime_dir(&self) -> PathBuf {
        match &self.root {
            Some(root) => root.join("home/.run/win11-gnome"),
            None => glib::user_runtime_dir().join("win11-gnome"),
        }
    }
    /// The program for `tool`: the system path, or in test mode only an executable stub inside the tool directory.
    fn tool(&self, tool: Tool) -> Option<PathBuf> {
        let (name, system) = TOOLS
            .iter()
            .find(|(t, _, _)| *t == tool)
            .map(|(_, n, p)| (*n, *p))?;
        if !self.test {
            return Some(PathBuf::from(system));
        }
        let dir = self.tools.as_ref()?;
        let stub = dir.join(name);
        let inside = stub
            .canonicalize()
            .ok()
            .zip(dir.canonicalize().ok())
            .is_some_and(|(s, d)| s.starts_with(d));
        let executable =
            fs::metadata(&stub).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
        (inside && executable).then_some(stub)
    }
    /// The only place this file builds a command: cleared environment, `LANG=C.UTF-8`, the table's `PATH`; in test
    /// mode the stub directory as `PATH` and dead bus addresses inside it.
    fn command(&self, tool: Tool) -> Option<Command> {
        let mut command = Command::new(self.tool(tool)?);
        command.env_clear().env("LANG", "C.UTF-8");
        for (key, value) in self.child_env() {
            command.env(key, value);
        }
        Some(command)
    }
    fn child_env(&self) -> Vec<(&'static str, String)> {
        match (&self.tools, self.test) {
            (Some(dir), true) => {
                let dead = |name: &str| format!("unix:path={}", dir.join(name).display());
                vec![
                    ("PATH", dir.display().to_string()),
                    ("DBUS_SYSTEM_BUS_ADDRESS", dead(NO_SYSTEM_BUS)),
                    ("DBUS_SESSION_BUS_ADDRESS", dead(NO_SESSION_BUS)),
                ]
            }
            (_, true) => Vec::new(),
            _ => vec![("PATH", SYSTEM_PATH.to_owned())],
        }
    }
    /// test mode waits shorter (the ShellVersion retries and the notification wait), so the layer test stays quick
    fn shell_wait(&self) -> Duration {
        Duration::from_secs(if self.test { 2 } else { 30 })
    }
    fn action_wait(&self) -> Duration {
        Duration::from_secs(if self.test { 8 } else { 600 })
    }
}

/// One program with a timeout: (exit code, stdout).
fn run_tool(setup: &Setup, tool: Tool, args: &[&str]) -> Option<(i32, String)> {
    let mut command = setup.command(tool)?;
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = (&mut stdout).take(1 << 20).read_to_string(&mut text);
        text
    });
    let started = Instant::now();
    let rc = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) if started.elapsed() < Duration::from_secs(20) => {
                std::thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let text = reader.join().unwrap_or_default();
    Some((rc?, text))
}

// ------------------------------------------------------------------------------------------------ pure parts

/// `VERSION_ID`, `VERSION_CODENAME` and `VERSION` (without its " (Codename)").
fn os_release(text: &str) -> (Option<String>, Option<String>, Option<String>) {
    let value = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
            .map(|v| v.trim().trim_matches('"').to_owned())
            .filter(|v| !v.is_empty())
    };
    let version = value("VERSION").map(|v| match v.rsplit_once(" (") {
        Some((head, tail)) if tail.ends_with(')') => head.trim().to_owned(),
        _ => v,
    });
    (value("VERSION_ID"), value("VERSION_CODENAME"), version)
}
/// The GNOME major of "50.1" / "1:51.0-0ubuntu1" (digits before the first '.').
fn major(version: &str) -> Option<String> {
    let version = version.trim();
    let version = version.split_once(':').map_or(version, |(_, rest)| rest);
    let digits: String = version.chars().take_while(char::is_ascii_digit).collect();
    (!digits.is_empty()).then_some(digits)
}
/// A changed release or GNOME major against the record (an unknown major on either side is no change).
fn changed(old: &Value, now: &Value) -> bool {
    old["versionId"] != now["versionId"]
        || old["codename"] != now["codename"]
        || (old["gnomeMajor"].is_string()
            && now["gnomeMajor"].is_string()
            && old["gnomeMajor"] != now["gnomeMajor"])
}

#[derive(Clone, Debug, PartialEq)]
enum Finding {
    Upgraded,
    ExtOff,
    ExtError,
    ExtsOff,
    /// the labels' catalogue rows and the old package names of the missing parts
    Missing {
        labels: Vec<&'static str>,
        packages: Vec<&'static str>,
    },
}
/// Step 2 (pure): `disable-user-extensions`, then the extension's state (a double, rounded; missing → ERROR), and
/// whether our uuid is in `enabled-extensions`.
fn extension_finding(
    disable_user: Option<bool>,
    state: Option<f64>,
    enabled: bool,
) -> Option<Finding> {
    if disable_user == Some(true) {
        return Some(Finding::ExtsOff);
    }
    match state.filter(|s| s.is_finite()).map(f64::round) {
        Some(s) if s == 1.0 => Some(Finding::Upgraded),
        Some(s) if s == 4.0 => Some(Finding::ExtOff),
        Some(s) if s == 2.0 && enabled => Some(Finding::ExtOff),
        Some(s) if s == 3.0 => Some(Finding::ExtError),
        None => Some(Finding::ExtError),
        Some(_) => None,
    }
}
/// Step 3: the blocking sonames that are not on disk (a dangling link counts as missing).
fn missing_parts(soname_dir: &Path) -> Option<Finding> {
    let (mut labels, mut packages) = (Vec::new(), Vec::new());
    for (soname, package, label) in PARTS {
        if !soname_dir.join(soname).exists() {
            if !labels.contains(label) {
                labels.push(*label);
            }
            if !packages.contains(package) {
                packages.push(*package);
            }
        }
    }
    (!labels.is_empty()).then_some(Finding::Missing { labels, packages })
}
/// A Debian package name as apt accepts it.
fn valid_name(name: &str) -> bool {
    let b = name.as_bytes();
    (2..=128).contains(&b.len())
        && b[0].is_ascii_alphanumeric()
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"+-.".contains(c))
}
/// `apt-cache policy <name>`: the candidate (None: "(none)" or the package is unknown).
fn policy_candidate(text: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.trim().strip_prefix("Candidate:"))
        .map(str::trim)
        .filter(|v| !v.is_empty() && *v != "(none)")
        .map(str::to_owned)
}
/// `apt-cache showpkg <name>`: the packages under "Reverse Provides:" (distinct, in order).
fn reverse_provides(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if line.starts_with("Reverse Provides:") {
            inside = true;
            continue;
        }
        if inside {
            match line.split_whitespace().next() {
                Some(name) if valid_name(name) => {
                    if !out.iter().any(|o| o == name) {
                        out.push(name.to_owned());
                    }
                }
                Some(_) => break,
                None => {}
            }
        }
    }
    out
}
/// Ubuntu's release upgrader (the rule of `services/update/guard.rs` `is_upgrader`).
fn is_upgrader(args: &[String]) -> bool {
    let Some(first) = args.first() else {
        return false;
    };
    let base = |a: &str| a.rsplit('/').next().unwrap_or(a).to_owned();
    let upgrader =
        |a: &str| base(a) == "do-release-upgrade" || a.starts_with("/tmp/ubuntu-release-upgrader-");
    let program = base(first);
    if program.starts_with("python") {
        let mut at = 1;
        while let Some(arg) = args.get(at) {
            match arg.as_str() {
                "-c" | "-m" | "-" => return false,
                "-W" | "-X" | "-Q" => at += 2,
                option if option.starts_with('-') => at += 1,
                script => return upgrader(script),
            }
        }
        return false;
    }
    if program == "screen" || program == "SCREEN" {
        return args
            .windows(2)
            .any(|p| p[0] == "-S" && p[1] == UPGRADE_SCREEN);
    }
    upgrader(first)
}
fn upgrader_running(proc: &Path) -> bool {
    let Ok(entries) = fs::read_dir(proc) else {
        return false;
    };
    entries
        .flatten()
        .filter(|e| e.file_name().as_bytes().iter().all(u8::is_ascii_digit))
        .any(|e| {
            let mut bytes = Vec::new();
            if let Ok(file) = fs::File::open(e.path().join("cmdline")) {
                let _ = file.take(64 << 10).read_to_end(&mut bytes);
            }
            let args: Vec<String> = bytes
                .split(|b| *b == 0)
                .filter(|a| !a.is_empty())
                .map(|a| String::from_utf8_lossy(a).into_owned())
                .collect();
            is_upgrader(&args)
        })
}

// ------------------------------------------------------------------------------------------------ files

fn config_dir() -> PathBuf {
    crate::i18n::config_dir()
}
fn cache_dir() -> PathBuf {
    glib::user_cache_dir().join("win11-gnome/update")
}
/// A small JSON file of ours (regular, not a link, our uid).
fn read_json(path: &Path) -> Option<Value> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } || meta.len() > 1 << 16 {
        return None;
    }
    let mut text = String::new();
    file.take(1 << 16).read_to_string(&mut text).ok()?;
    serde_json::from_str(&text).ok()
}
/// Temp file 0600 in the same directory (a 0700 one of ours, created when missing), then rename.
fn write_json(path: &Path, value: &Value) -> bool {
    let Some(dir) = path.parent() else {
        return false;
    };
    if fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .is_err()
    {
        return false;
    }
    let temp = dir.join(format!(".{}-{}.tmp", RECORD, std::process::id()));
    let written = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temp)?;
        file.write_all(value.to_string().as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written.is_ok()
}
/// Step 6: the cached release state of the old release goes.
fn clean_caches(setup: &Setup) {
    let dir = cache_dir();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let ours = (name.starts_with("compat-") && name.ends_with(".json"))
                || name == "release.json"
                || name == "upgrade.json";
            if ours
                && entry
                    .file_type()
                    .is_ok_and(|t| t.is_file() || t.is_symlink())
            {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    for name in ["upgrade-status.json", "attach-status.json"] {
        let _ = fs::remove_file(setup.runtime_dir().join(name));
    }
}
fn extension_version() -> Value {
    let path = glib::user_data_dir()
        .join("gnome-shell/extensions")
        .join(UUID)
        .join("metadata.json");
    fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .map(|m| m["version"].clone())
        .unwrap_or(Value::Null)
}

// ------------------------------------------------------------------------------------------------ GNOME

fn call(
    bus: &gio::DBusConnection,
    name: &str,
    path: &str,
    iface: &str,
    method: &str,
    args: Option<&glib::Variant>,
    reply: &str,
) -> Option<glib::Variant> {
    let reply = glib::VariantTy::new(reply).ok()?;
    bus.call_sync(
        Some(name),
        path,
        iface,
        method,
        args,
        Some(reply),
        gio::DBusCallFlags::NO_AUTO_START,
        5000,
        gio::Cancellable::NONE,
    )
    .ok()
}
/// `org.gnome.Shell` `ShellVersion`, retried for up to `wait` (the shell may still be starting at autostart time).
fn shell_version(bus: Option<&gio::DBusConnection>, wait: Duration) -> Option<String> {
    let bus = bus?;
    let started = Instant::now();
    loop {
        let args = ("org.gnome.Shell", "ShellVersion").to_variant();
        let reply = call(
            bus,
            "org.gnome.Shell",
            "/org/gnome/Shell",
            "org.freedesktop.DBus.Properties",
            "Get",
            Some(&args),
            "(v)",
        );
        if let Some(version) = reply
            .and_then(|r| r.child_value(0).as_variant())
            .and_then(|v| v.str().map(str::to_owned))
        {
            return Some(version);
        }
        if started.elapsed() >= wait {
            return None;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}
fn number(value: &glib::Variant) -> Option<f64> {
    match value.type_().as_str() {
        "d" => value.get::<f64>(),
        "i" => value.get::<i32>().map(f64::from),
        "u" => value.get::<u32>().map(f64::from),
        "x" => value.get::<i64>().map(|v| v as f64),
        "t" => value.get::<u64>().map(|v| v as f64),
        "n" => value.get::<i16>().map(f64::from),
        "q" => value.get::<u16>().map(f64::from),
        "y" => value.get::<u8>().map(f64::from),
        _ => None,
    }
}
/// `GetExtensionInfo(uuid)` → `state` (GNOME packs numbers as doubles).
fn extension_state(bus: Option<&gio::DBusConnection>) -> Option<f64> {
    let reply = call(
        bus?,
        "org.gnome.Shell.Extensions",
        "/org/gnome/Shell/Extensions",
        "org.gnome.Shell.Extensions",
        "GetExtensionInfo",
        Some(&(UUID,).to_variant()),
        "(a{sv})",
    )?;
    let dict = glib::VariantDict::new(Some(&reply.child_value(0)));
    number(&dict.lookup_value("state", None)?)
}
/// org.gnome.shell's settings: the default source and backend; in test mode only the fixture root's compiled
/// schemas and a keyfile in it (never dconf).
fn shell_settings(setup: &Setup) -> Option<gio::Settings> {
    let (schema, backend) = match &setup.root {
        Some(root) => {
            let dir = root.join("usr/share/glib-2.0/schemas");
            if !dir.join("gschemas.compiled").is_file() {
                return None;
            }
            let source = gio::SettingsSchemaSource::from_directory(&dir, None, false).ok()?;
            let keyfile = root.join("home/.config/glib-2.0/settings/keyfile");
            (
                source.lookup(SHELL_SCHEMA, false)?,
                Some(gio::keyfile_settings_backend_new(
                    keyfile.to_str()?,
                    "/",
                    None,
                )),
            )
        }
        None => (
            gio::SettingsSchemaSource::default()?.lookup(SHELL_SCHEMA, true)?,
            None,
        ),
    };
    let typed =
        |key: &str, ty: &str| schema.has_key(key) && schema.key(key).value_type().as_str() == ty;
    if !typed(KEY_DISABLE, "b") || !typed(KEY_ENABLED, "as") {
        return None;
    }
    Some(gio::Settings::new_full(&schema, backend.as_ref(), None))
}

// ------------------------------------------------------------------------------------------------ the check

/// What a notification offers: its id and the action keys it may answer with.
struct Shown {
    id: u32,
    actions: Vec<&'static str>,
}
struct Check {
    setup: Setup,
    lang: crate::i18n::Lang,
    bus: Option<gio::DBusConnection>,
    settings: Option<gio::Settings>,
    major: Option<String>,
}

fn product(lang: crate::i18n::Lang) -> String {
    crate::i18n::tr(lang, "update.product")
}
/// `LANGUAGE` of the parts terminal (as upgrade.rs: `nl:en` for Dutch, `en` for English).
fn language_list(lang: crate::i18n::Lang) -> String {
    let (code, fallback) = (lang.code(), crate::i18n::Lang::En.code());
    if code == fallback {
        code.to_owned()
    } else {
        [code, fallback].join(":")
    }
}

impl Check {
    fn text(&self, key: &str, params: &Value) -> String {
        let mut params = params.as_object().cloned().unwrap_or_default();
        params
            .entry("product")
            .or_insert_with(|| json!(product(self.lang)));
        crate::i18n::trf(self.lang, key, &Value::Object(params))
    }
    /// One GNOME notification (app name {product}, our Settings' desktop entry); its id.
    fn notify(&self, title: &str, body: &str, actions: &[&'static str]) -> Option<u32> {
        let bus = self.bus.as_ref()?;
        let mut list: Vec<String> = Vec::new();
        for action in actions {
            let label = match *action {
                "dockOn" => "update.post.dockOn",
                "extsOn" => "update.post.extsOn",
                "installParts" => "update.post.installParts",
                _ => "update.post.later",
            };
            list.push((*action).to_owned());
            list.push(self.text(label, &Value::Null));
        }
        let hints = glib::VariantDict::new(None);
        hints.insert("desktop-entry", "io.github.win11shell.Settings");
        let args = glib::Variant::tuple_from_iter([
            product(self.lang).to_variant(),
            0u32.to_variant(),
            "system-software-update".to_variant(),
            title.to_variant(),
            body.to_variant(),
            list.to_variant(),
            hints.end(),
            (-1i32).to_variant(),
        ]);
        let reply = call(
            bus,
            "org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
            "Notify",
            Some(&args),
            "(u)",
        )?;
        reply.child_value(0).get::<u32>()
    }
    fn show(&self, finding: &Finding) -> Option<Shown> {
        let (title, body, actions): (String, String, Vec<&'static str>) = match finding {
            Finding::Upgraded => {
                let (id, _, version) =
                    os_release(&fs::read_to_string(self.setup.at(OS_RELEASE)).unwrap_or_default());
                let version = version.or(id).unwrap_or_default();
                (
                    self.text("update.post.upgraded", &json!({"version":version})),
                    String::new(),
                    Vec::new(),
                )
            }
            Finding::ExtOff => {
                let major = self.major.clone().unwrap_or_default();
                (
                    self.text("update.post.extOff", &json!({"major":major})),
                    self.text("update.post.extOffBody", &Value::Null),
                    vec!["dockOn", "later"],
                )
            }
            Finding::ExtError => (
                self.text("update.post.extError", &Value::Null),
                self.text("update.post.extErrorBody", &Value::Null),
                vec!["dockOn", "later"],
            ),
            Finding::ExtsOff => (
                self.text("update.post.extsOff", &Value::Null),
                self.text("update.post.extsOffBody", &Value::Null),
                vec!["extsOn", "dockOn"],
            ),
            Finding::Missing { labels, .. } => {
                let parts: Vec<String> = labels
                    .iter()
                    .map(|key| crate::i18n::tr(self.lang, key))
                    .collect();
                let body = self.text(
                    "update.post.missingBody",
                    &json!({"parts":parts.join(", ")}),
                );
                (
                    self.text("update.post.missing", &Value::Null),
                    body,
                    vec!["installParts", "later"],
                )
            }
        };
        let id = self.notify(&title, &body, &actions)?;
        Some(Shown { id, actions })
    }
    /// An action of one of our notifications, only after its click (design §6.6 step 4).
    fn act(&self, action: &str, finding: &Finding, token: Option<&str>) {
        match action {
            "dockOn" => {
                if let Some(bus) = &self.bus {
                    call(
                        bus,
                        "org.gnome.Shell.Extensions",
                        "/org/gnome/Shell/Extensions",
                        "org.gnome.Shell.Extensions",
                        "EnableExtension",
                        Some(&(DOCK,).to_variant()),
                        "(b)",
                    );
                }
            }
            "extsOn" => {
                if let Some(settings) = &self.settings {
                    let _ = settings.set_boolean(KEY_DISABLE, false);
                    if !self.setup.test {
                        gio::Settings::sync();
                    }
                }
            }
            "installParts" => {
                if let Finding::Missing { packages, .. } = finding {
                    let names: Vec<String> = packages
                        .iter()
                        .map(|old| resolve_name(&self.setup, old))
                        .collect();
                    self.install_parts(&names, token);
                }
            }
            _ => {}
        }
    }
    /// `xdg-terminal-exec -- <sh> -c '<WRAPPER>' win11-gnome-parts <status> always <pkexec> <apt-get> install <names>`
    /// through `process_scope::spawn` (Role::App, `app.slice`).
    fn install_parts(&self, names: &[String], token: Option<&str>) {
        let setup = &self.setup;
        let (Some(sh), Some(pkexec), Some(apt_get), Some(mut command)) = (
            setup.tool(Tool::Sh),
            setup.tool(Tool::Pkexec),
            setup.tool(Tool::AptGet),
            setup
                .tool(Tool::XdgTerminalExec)
                .filter(|p| p.is_file())
                .and_then(|_| setup.command(Tool::XdgTerminalExec)),
        ) else {
            return;
        };
        let dir = setup.runtime_dir();
        if fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .is_err()
            || !fs::symlink_metadata(&dir).is_ok_and(|m| {
                m.is_dir() && m.uid() == unsafe { libc::geteuid() } && m.mode() & 0o077 == 0
            })
        {
            return;
        }
        let status = dir.join(PARTS_STATUS);
        let _ = fs::remove_file(&status);
        let home = setup.home();
        let private = |key: &str| {
            key.starts_with("WIN11SHELL_")
                || key.starts_with("WIN11_DEV_")
                || key.starts_with("WIN11_TEST_")
                || key.starts_with(VAR_PREFIX)
                || key == "WIN11_REFERENCE_INSTANCE"
                || key == "WIN11_UI_DIR"
        };
        let mut env: BTreeMap<OsString, OsString> = std::env::vars_os()
            .filter(|(k, _)| !private(&k.to_string_lossy()))
            .collect();
        env.insert("LANGUAGE".into(), language_list(self.lang).into());
        env.insert(
            "WIN11_PRESS_ENTER".into(),
            crate::i18n::tr(self.lang, "update.release.pressEnter").into(),
        );
        env.insert("PWD".into(), home.as_os_str().to_owned());
        env.remove(std::ffi::OsStr::new("DESKTOP_STARTUP_ID"));
        env.remove(std::ffi::OsStr::new("XDG_ACTIVATION_TOKEN"));
        if let Some(token) = token {
            env.insert("DESKTOP_STARTUP_ID".into(), token.into());
            env.insert("XDG_ACTIVATION_TOKEN".into(), token.into());
        }
        if setup.test {
            for (key, value) in setup.child_env() {
                env.insert(key.into(), value.into());
            }
        }
        command.env_clear().envs(&env).current_dir(&home);
        command
            .arg("--")
            .arg(sh)
            .arg("-c")
            .arg(WRAPPER)
            .arg(PARTS_NAME)
            .arg(&status)
            .arg("always")
            .arg(pkexec)
            .arg(apt_get)
            .arg("install");
        command.args(names);
        let env: Vec<(OsString, OsString)> = env.into_iter().collect();
        let _ = crate::process_scope::spawn(command, "post-upgrade-parts", Some(env));
    }
    /// Show the findings and wait (≤ 10 minutes) until each notification with actions was answered or closed.
    fn notify_and_wait(&self, findings: Vec<Finding>) {
        let Some(bus) = self.bus.clone() else { return };
        #[derive(Default)]
        struct Waiting {
            open: Vec<(u32, Vec<&'static str>, Finding)>,
            tokens: BTreeMap<u32, String>,
            answers: Vec<(u32, String)>,
            closed: Vec<u32>,
        }
        let state = Rc::new(RefCell::new(Waiting::default()));
        let main = glib::MainLoop::new(None, false);
        let subscription = {
            let (state, main) = (state.clone(), main.clone());
            bus.signal_subscribe(
                Some("org.freedesktop.Notifications"),
                Some("org.freedesktop.Notifications"),
                None,
                Some("/org/freedesktop/Notifications"),
                None,
                gio::DBusSignalFlags::NONE,
                move |_, _, _, _, member, params| {
                    let mut s = state.borrow_mut();
                    let id = params
                        .try_child_value(0)
                        .and_then(|v| v.get::<u32>())
                        .unwrap_or(0);
                    match member {
                        "ActivationToken" => {
                            if let Some(token) = params
                                .try_child_value(1)
                                .and_then(|v| v.str().map(str::to_owned))
                            {
                                s.tokens.insert(id, token);
                            }
                        }
                        "ActionInvoked" => {
                            if let Some(key) = params
                                .try_child_value(1)
                                .and_then(|v| v.str().map(str::to_owned))
                            {
                                s.answers.push((id, key));
                            }
                            main.quit();
                        }
                        "NotificationClosed" => {
                            s.closed.push(id);
                            main.quit();
                        }
                        _ => {}
                    }
                },
            )
        };
        for finding in findings {
            if let Some(shown) = self.show(&finding)
                && !shown.actions.is_empty()
            {
                state
                    .borrow_mut()
                    .open
                    .push((shown.id, shown.actions, finding));
            }
        }
        let deadline = Instant::now() + self.setup.action_wait();
        while !state.borrow().open.is_empty() && Instant::now() < deadline {
            let timer = {
                let main = main.clone();
                glib::timeout_add_local_once(
                    deadline.saturating_duration_since(Instant::now()),
                    move || main.quit(),
                )
            };
            main.run();
            timer.remove_if_pending();
            let (answers, closed, tokens) = {
                let mut s = state.borrow_mut();
                (
                    std::mem::take(&mut s.answers),
                    std::mem::take(&mut s.closed),
                    s.tokens.clone(),
                )
            };
            for (id, key) in answers {
                let open = state
                    .borrow()
                    .open
                    .iter()
                    .position(|(o, actions, _)| *o == id && actions.contains(&key.as_str()));
                if let Some(at) = open {
                    let (_, _, finding) = state.borrow_mut().open.remove(at);
                    self.act(&key, &finding, tokens.get(&id).map(String::as_str));
                }
            }
            state
                .borrow_mut()
                .open
                .retain(|(id, _, _)| !closed.contains(id));
        }
        bus.signal_unsubscribe(subscription);
    }
}

trait RemoveIfPending {
    fn remove_if_pending(self);
}
impl RemoveIfPending for glib::SourceId {
    /// A one-shot timeout that already fired is gone; removing it again would be a GLib critical.
    fn remove_if_pending(self) {
        if glib::MainContext::default()
            .find_source_by_id(&self)
            .is_some()
        {
            self.remove();
        }
    }
}

/// The name to install for a missing part on the new release: the old name while it has a candidate, else the
/// single package that provides it, else the old name (apt then explains in the held window).
fn resolve_name(setup: &Setup, old: &str) -> String {
    let candidate = run_tool(setup, Tool::AptCache, &["policy", old])
        .and_then(|(rc, text)| (rc == 0).then(|| policy_candidate(&text)).flatten());
    if candidate.is_some() {
        return old.to_owned();
    }
    let provides = run_tool(setup, Tool::AptCache, &["showpkg", old])
        .map(|(_, text)| reverse_provides(&text))
        .unwrap_or_default();
    match provides.as_slice() {
        [one] => one.clone(),
        _ => old.to_owned(),
    }
}
/// Step 5: `loginctl disable-linger <uid>` while our stamp exists (no upgrader runs: checked by the caller).
fn linger_cleanup(setup: &Setup) {
    let stamp = config_dir().join(LINGER_STAMP);
    if !stamp.is_file() {
        return;
    }
    let uid = unsafe { libc::getuid() }.to_string();
    if run_tool(setup, Tool::Loginctl, &["disable-linger", &uid]).is_some_and(|(rc, _)| rc == 0) {
        let _ = fs::remove_file(stamp);
    }
}

/// `win11-launch --post-upgrade-check`; the exit code (0 also when nothing could be done).
pub(crate) fn run() -> i32 {
    let vars: Vec<(String, OsString)> = std::env::vars_os()
        .filter_map(|(k, v)| {
            k.to_str()
                .filter(|k| k.starts_with(VAR_PREFIX))
                .map(|k| (k.to_owned(), v))
        })
        .collect();
    let setup = match setup_from(&vars, unsafe { libc::geteuid() }) {
        Ok(setup) => setup,
        Err(reason) => {
            eprintln!("win11-launch {FLAG}: test seam refused ({reason}), nothing done");
            return 0;
        }
    };
    check(setup);
    0
}
fn check(setup: Setup) {
    // while Ubuntu's upgrader runs (its files may already be the new release's), the next login checks
    if upgrader_running(&setup.at(PROC)) {
        return;
    }
    let (version_id, codename, _) =
        os_release(&fs::read_to_string(setup.at(OS_RELEASE)).unwrap_or_default());
    let bus = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).ok();
    let major = shell_version(bus.as_ref(), setup.shell_wait())
        .or_else(|| {
            run_tool(
                &setup,
                Tool::DpkgQuery,
                &["-W", "-f=${Version}", "gnome-shell"],
            )
            .filter(|(rc, _)| *rc == 0)
            .map(|(_, v)| v)
        })
        .and_then(|v| major(&v));
    let now = json!({"versionId":version_id,"codename":codename,"gnomeMajor":major.clone(),"extensionVersion":extension_version()});
    let record_path = config_dir().join(RECORD);
    let record = read_json(&record_path).filter(Value::is_object);
    let Some(record) = record else {
        linger_cleanup(&setup);
        write_json(&record_path, &now);
        return;
    };
    if !changed(&record, &now) {
        linger_cleanup(&setup);
        return;
    }
    let settings = shell_settings(&setup);
    let check = Check {
        lang: crate::i18n::current_language(),
        bus,
        settings,
        major,
        setup,
    };
    let mut findings = Vec::new();
    let disable_user = check.settings.as_ref().map(|s| s.boolean(KEY_DISABLE));
    let enabled = check
        .settings
        .as_ref()
        .is_some_and(|s| s.strv(KEY_ENABLED).iter().any(|u| u.as_str() == UUID));
    let state = if disable_user == Some(true) {
        None
    } else {
        extension_state(check.bus.as_ref())
    };
    match extension_finding(disable_user, state, enabled) {
        // without a known major the "GNOME {major} turned … off" title cannot be written: the error text instead
        Some(Finding::ExtOff) if check.major.is_none() => findings.push(Finding::ExtError),
        Some(finding) => findings.push(finding),
        None => {}
    }
    findings.extend(missing_parts(&check.setup.at(SONAME_DIR)));
    linger_cleanup(&check.setup);
    check.notify_and_wait(findings);
    write_json(&record_path, &now);
    clean_caches(&check.setup);
}

// ------------------------------------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/update/fixtures/upgrade")
            .join(name);
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }
    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn private_dir(name: &str) -> Dir {
        let dir = std::env::temp_dir().join(format!(
            "w11pu-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .unwrap();
        fs::write(dir.join(MARKER), b"").unwrap();
        Dir(dir)
    }
    fn script(path: &Path, body: &str) {
        fs::write(path, format!("#!/bin/sh\n{body}")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// The decision table of step 2 with states as doubles (design §9.2).
    #[test]
    fn the_extension_state_is_read_as_a_double() {
        let cases: [(Option<bool>, Option<f64>, bool, Option<Finding>); 13] = [
            (Some(true), Some(1.0), true, Some(Finding::ExtsOff)),
            (Some(true), None, false, Some(Finding::ExtsOff)),
            (Some(false), Some(1.0), true, Some(Finding::Upgraded)),
            (None, Some(1.0), false, Some(Finding::Upgraded)),
            (Some(false), Some(4.0), false, Some(Finding::ExtOff)),
            (Some(false), Some(3.9), false, Some(Finding::ExtOff)),
            (Some(false), Some(2.0), true, Some(Finding::ExtOff)),
            (Some(false), Some(2.0), false, None),
            (Some(false), Some(3.0), true, Some(Finding::ExtError)),
            (Some(false), None, true, Some(Finding::ExtError)),
            (Some(false), Some(f64::NAN), true, Some(Finding::ExtError)),
            (Some(false), Some(6.0), true, None),
            (Some(false), Some(99.0), true, None),
        ];
        for (disable, state, enabled, want) in cases {
            assert_eq!(
                extension_finding(disable, state, enabled),
                want,
                "{disable:?} {state:?} {enabled}"
            );
        }
        for (ty, value, want) in [
            ("d", 4.0f64.to_variant(), Some(4.0)),
            ("i", 2i32.to_variant(), Some(2.0)),
            ("u", 3u32.to_variant(), Some(3.0)),
            ("s", "1".to_variant(), None),
        ] {
            assert_eq!(number(&value), want, "{ty}");
        }
    }

    #[test]
    fn releases_majors_and_the_record() {
        let (id, codename, version) = os_release(
            "NAME=\"Ubuntu\"\nVERSION_ID=\"26.10\"\nVERSION=\"26.10 (Stonking Stingray)\"\nVERSION_CODENAME=stonking\n",
        );
        assert_eq!(
            (id.as_deref(), codename.as_deref(), version.as_deref()),
            (Some("26.10"), Some("stonking"), Some("26.10"))
        );
        let (_, _, lts) = os_release("VERSION=\"26.04.1 LTS (Resolute Raccoon)\"\n");
        assert_eq!(lts.as_deref(), Some("26.04.1 LTS"));
        assert_eq!(major("50.1"), Some("50".into()));
        assert_eq!(major("1:51.0-0ubuntu1\n"), Some("51".into()));
        assert_eq!(major("alpha"), None);
        let at = |v: &str, c: &str, m: Value| json!({"versionId":v,"codename":c,"gnomeMajor":m,"extensionVersion":3});
        assert!(!changed(
            &at("26.04", "resolute", json!("50")),
            &at("26.04", "resolute", json!("50"))
        ));
        assert!(changed(
            &at("26.04", "resolute", json!("50")),
            &at("26.10", "stonking", json!("51"))
        ));
        assert!(
            changed(
                &at("26.04", "resolute", json!("50")),
                &at("26.04", "resolute", json!("51"))
            ),
            "a GNOME major alone"
        );
        assert!(
            !changed(
                &at("26.04", "resolute", json!("50")),
                &at("26.04", "resolute", Value::Null)
            ),
            "an unknown major is no change"
        );
    }

    /// apt-cache outputs (fixtures in the shape apt 3 prints): the candidate, Reverse Provides, and the name rule.
    #[test]
    fn apt_cache_outputs_resolve_the_names() {
        assert_eq!(
            policy_candidate(&fixture("apt/policy-libgtk-4-1.txt")).as_deref(),
            Some("4.24.1+ds-1ubuntu1")
        );
        assert_eq!(
            policy_candidate(&fixture("apt/policy-libgtk-3-0t64.txt")),
            None,
            "Candidate: (none)"
        );
        assert_eq!(
            policy_candidate(""),
            None,
            "an unknown package prints nothing"
        );
        assert_eq!(
            reverse_provides(&fixture("apt/showpkg-libgtk-3-0t64.txt")),
            ["libgtk-3-0t65"]
        );
        assert_eq!(
            reverse_provides(&fixture("apt/showpkg-libarchive13t64.txt")),
            ["libarchive13t65", "libarchive-ng13"]
        );
        assert!(reverse_provides("Package: x\nReverse Provides: \n").is_empty());
        // through the tool table's apt-cache stub
        let tools = private_dir("apt-tools");
        let root = private_dir("apt-root");
        let dir =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/update/fixtures/upgrade/apt");
        script(
            &tools.0.join("apt-cache"),
            &format!(
                "f='{}'/\"$1-$2.txt\"\n[ -f \"$f\" ] || exit 0\nwhile IFS= read -r l; do printf '%s\\n' \"$l\"; done < \"$f\"\n",
                dir.display()
            ),
        );
        let vars = vec![
            (
                "WIN11_UPDATE_FIXTURE_ROOT".to_owned(),
                root.0.clone().into_os_string(),
            ),
            (
                "WIN11_UPDATE_TOOL_DIR".to_owned(),
                tools.0.clone().into_os_string(),
            ),
        ];
        let setup = setup_from(&vars, unsafe { libc::geteuid() }).unwrap();
        assert_eq!(
            resolve_name(&setup, "libgtk-4-1"),
            "libgtk-4-1",
            "a candidate: the old name"
        );
        assert_eq!(
            resolve_name(&setup, "libgtk-3-0t64"),
            "libgtk-3-0t65",
            "no candidate, one provider"
        );
        assert_eq!(
            resolve_name(&setup, "libarchive13t64"),
            "libarchive13t64",
            "two providers: the old name"
        );
        assert_eq!(
            resolve_name(&setup, "zlib1g"),
            "zlib1g",
            "unknown everywhere: the old name"
        );
    }

    /// The blocking sonames on a fixture soname directory; the table covers every NEEDED of the `BINS`.
    #[test]
    fn missing_parts_come_from_the_soname_directory() {
        let dir = private_dir("sonames");
        for (soname, _, _) in PARTS {
            fs::write(dir.0.join(soname), b"").unwrap();
        }
        assert_eq!(missing_parts(&dir.0), None);
        fs::remove_file(dir.0.join("libgdk_pixbuf-2.0.so.0")).unwrap();
        fs::remove_file(dir.0.join("libgtk-4.so.1")).unwrap();
        assert_eq!(
            missing_parts(&dir.0),
            Some(Finding::Missing {
                labels: vec!["update.part.gtk4", "update.part.pixbuf"],
                packages: vec!["libgtk-4-1", "libgdk-pixbuf-2.0-0"]
            })
        );
        // GTK 3 is no part of ours (no binary links it since 14-11): its absence is no finding
        assert!(
            !PARTS
                .iter()
                .any(|(soname, _, _)| soname.starts_with("libgtk-3")
                    || soname.starts_with("libgdk-3"))
        );
        fs::remove_file(dir.0.join("libz.so.1")).unwrap();
        std::os::unix::fs::symlink(dir.0.join("gone"), dir.0.join("libz.so.1")).unwrap();
        assert!(
            matches!(missing_parts(&dir.0), Some(Finding::Missing { labels, .. }) if labels.contains(&"update.part.zlib")),
            "a dangling link"
        );
        // every NEEDED soname of the build's binaries (minus libc, libm, libgcc_s, ld-linux) is in the table
        let build = build_dir();
        let mut seen = 0;
        for name in bins() {
            let path = build.join(&name);
            let bytes = fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            for soname in needed(&bytes) {
                if soname.starts_with("libc.so")
                    || soname.starts_with("libm.so")
                    || soname.starts_with("libgcc_s")
                    || soname.starts_with("ld-linux")
                {
                    continue;
                }
                assert!(
                    PARTS.iter().any(|(s, _, _)| *s == soname),
                    "{soname} of {} is not in PARTS (or the build is older than this source)",
                    path.display()
                );
                seen += 1;
            }
        }
        assert!(seen > 20, "the build's binaries were read");
    }
    /// The program names of scripts/install.py `BINS` (every binary an install has).
    fn bins() -> Vec<String> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/install.py");
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let list = text
            .split("\nBINS = (")
            .nth(1)
            .and_then(|t| t.split(')').next())
            .expect("install.py BINS");
        let names: Vec<String> = list
            .split('"')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect();
        assert!(names.iter().all(|n| n.starts_with("win11-")), "{names:?}");
        names
    }
    /// A build directory with every `BINS` binary: `W11_TEST_BUILD_DIR`, else tests/build/native-release. A missing
    /// binary fails the test (no silent skip).
    fn build_dir() -> PathBuf {
        let dir = std::env::var_os("W11_TEST_BUILD_DIR").map_or_else(
            || Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/build/native-release"),
            PathBuf::from,
        );
        let missing: Vec<String> = bins()
            .into_iter()
            .filter(|name| !dir.join(name).is_file())
            .collect();
        assert!(
            missing.is_empty(),
            "{} lacks {missing:?} (W11_TEST_BUILD_DIR names a build directory with all BINS)",
            dir.display()
        );
        dir
    }
    /// DT_NEEDED of a 64-bit little-endian ELF file (the reader of scripts/install.py `elf_needed`).
    fn needed(b: &[u8]) -> Vec<String> {
        let u16_at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]) as usize;
        let u64_at = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
        if b.len() < 64 || &b[..4] != b"\x7fELF" || b[4] != 2 || b[5] != 1 {
            return Vec::new();
        }
        let (phoff, phentsize, phnum) = (u64_at(0x20) as usize, u16_at(0x36), u16_at(0x38));
        let (mut loads, mut dynamic) = (Vec::new(), None);
        for i in 0..phnum {
            let p = phoff + i * phentsize;
            let kind = u32::from_le_bytes(b[p..p + 4].try_into().unwrap());
            let (offset, vaddr, filesz) = (u64_at(p + 8), u64_at(p + 16), u64_at(p + 32));
            match kind {
                1 => loads.push((vaddr, offset, filesz)),
                2 => dynamic = Some((offset as usize, filesz as usize)),
                _ => {}
            }
        }
        let Some((offset, size)) = dynamic else {
            return Vec::new();
        };
        let entries: Vec<(i64, u64)> = (0..size / 16)
            .map(|i| (u64_at(offset + i * 16) as i64, u64_at(offset + i * 16 + 8)))
            .collect();
        let Some(strtab) = entries.iter().find(|(t, _)| *t == 5).map(|(_, v)| *v) else {
            return Vec::new();
        };
        let Some(base) = loads
            .iter()
            .find(|(va, _, sz)| *va <= strtab && strtab < va + sz)
            .map(|(va, off, _)| (off + strtab - va) as usize)
        else {
            return Vec::new();
        };
        entries
            .iter()
            .filter(|(t, _)| *t == 1)
            .map(|(_, v)| {
                let start = base + *v as usize;
                let end = start + b[start..].iter().position(|c| *c == 0).unwrap_or(0);
                String::from_utf8_lossy(&b[start..end]).into_owned()
            })
            .collect()
    }

    /// Fail closed: an unknown variable, euid 0, a missing or unmarked fixture root, an unmarked tool dir.
    #[test]
    fn the_seams_fail_closed() {
        let root = private_dir("seam-root");
        let tools = private_dir("seam-tools");
        let me = unsafe { libc::geteuid() };
        let var = |k: &str, v: &Path| (k.to_owned(), v.as_os_str().to_owned());
        assert!(!setup_from(&[], me).unwrap().test, "production");
        let good = vec![
            var("WIN11_UPDATE_FIXTURE_ROOT", &root.0),
            var("WIN11_UPDATE_TOOL_DIR", &tools.0),
            var("WIN11_UPDATE_PK_BUS", Path::new("unix:path=x")),
        ];
        let setup = setup_from(&good, me).unwrap();
        assert!(
            setup.test
                && setup.at(OS_RELEASE).starts_with(&root.0)
                && setup.at(SONAME_DIR).starts_with(&root.0)
        );
        assert_eq!(
            setup.tool(Tool::Loginctl),
            None,
            "a missing stub: nothing, never a system path"
        );
        script(&tools.0.join("loginctl"), "exit 0\n");
        assert_eq!(setup.tool(Tool::Loginctl), Some(tools.0.join("loginctl")));
        let envs: Vec<(String, String)> = setup
            .command(Tool::Loginctl)
            .unwrap()
            .get_envs()
            .filter_map(|(k, v)| {
                Some((
                    k.to_string_lossy().into_owned(),
                    v?.to_string_lossy().into_owned(),
                ))
            })
            .collect();
        assert!(envs.contains(&("PATH".into(), tools.0.display().to_string())));
        let unmarked = private_dir("seam-unmarked");
        fs::remove_file(unmarked.0.join(MARKER)).unwrap();
        for (what, vars, euid) in [
            (
                "unknown variable",
                vec![var("WIN11_UPDATE_TYPO", &root.0)],
                me,
            ),
            ("euid 0", vec![var("WIN11_UPDATE_FIXTURE_ROOT", &root.0)], 0),
            (
                "no fixture root",
                vec![var("WIN11_UPDATE_TOOL_DIR", &tools.0)],
                me,
            ),
            (
                "unmarked root",
                vec![var("WIN11_UPDATE_FIXTURE_ROOT", &unmarked.0)],
                me,
            ),
            (
                "unmarked tools",
                vec![
                    var("WIN11_UPDATE_FIXTURE_ROOT", &root.0),
                    var("WIN11_UPDATE_TOOL_DIR", &unmarked.0),
                ],
                me,
            ),
            (
                "relative root",
                vec![(
                    "WIN11_UPDATE_FIXTURE_ROOT".to_owned(),
                    OsString::from("rel/dir"),
                )],
                me,
            ),
            (
                "root of another uid",
                vec![var("WIN11_UPDATE_FIXTURE_ROOT", &root.0)],
                me + 1,
            ),
        ] {
            assert!(setup_from(&vars, euid).is_err(), "{what}");
        }
        // production: absolute table paths, PATH=/usr/bin:/bin
        let production = Setup::default();
        assert!(production.tool(Tool::Pkexec).unwrap().is_absolute());
        assert_eq!(
            production.child_env(),
            vec![("PATH", SYSTEM_PATH.to_owned())]
        );
    }

    #[test]
    fn the_upgrader_rule_is_guards() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(is_upgrader(&a(&[
            "python3",
            "-s",
            "/x/do-release-upgrade",
            "-f",
            "DistUpgradeViewGtk3"
        ])));
        assert!(is_upgrader(&a(&[
            "python3.14",
            "-W",
            "ignore",
            "-u",
            "/tmp/ubuntu-release-upgrader-ab12/stonking"
        ])));
        assert!(is_upgrader(&a(&[
            "SCREEN",
            "-e",
            "\\0\\0",
            "-S",
            UPGRADE_SCREEN
        ])));
        assert!(is_upgrader(&a(&["do-release-upgrade"])));
        for line in [
            &["vim", "/tmp/ubuntu-release-upgrader-x/y"][..],
            &["python3", "-c", "x", "/tmp/ubuntu-release-upgrader-x/y"],
            &["screen", "-d", "-r", UPGRADE_SCREEN],
            &[
                "sh",
                "-c",
                "w",
                "win11-gnome-upgrade",
                "s",
                "fail",
                "pkexec",
                "do-release-upgrade",
            ],
        ] {
            assert!(!is_upgrader(&a(line)), "{line:?}");
        }
    }

    /// The texts come from update.tsv in the settings language (the bin's own i18n.rs).
    #[test]
    fn the_texts_follow_the_language() {
        use crate::i18n::{Lang, trf};
        assert_eq!(
            trf(
                Lang::Nl,
                "update.post.extOff",
                &json!({"major":"51","product":"Win11 Shell"})
            ),
            "GNOME 51 heeft Win11 Shell uitgezet"
        );
        assert_eq!(
            trf(
                Lang::En,
                "update.post.extOff",
                &json!({"major":"51","product":"Win11 Shell"})
            ),
            "GNOME 51 turned Win11 Shell off"
        );
        assert_eq!(
            trf(
                Lang::En,
                "update.post.missingBody",
                &json!({"parts":"GTK 4, GLib"})
            ),
            "Missing after the upgrade: GTK 4, GLib."
        );
        assert_eq!(language_list(Lang::Nl), "nl:en");
        assert_eq!(language_list(Lang::En), "en");
    }

    /// Design §4.11: no literal absolute exec path in this file outside its table, and no other way to start a
    /// program than the table's one command builder (the patterns are built so this test does not match itself).
    #[test]
    fn no_absolute_exec_path_outside_the_tool_table() {
        let text = include_str!("post_upgrade.rs");
        let (q, s) = ('"', '/');
        let command = ["Comm", "and"].concat();
        let paths: Vec<String> = [
            "usr/bin/",
            "usr/lib/",
            "bin/",
            "usr/libexec/",
            "sbin/",
            "usr/sbin/",
            "usr/local/",
        ]
        .iter()
        .map(|p| format!("{q}{s}{p}"))
        .chain([format!("{command}::new({q}{s}")])
        .collect();
        let starts: Vec<String> = [
            ("Sub", "process"),
            ("spawn_", "async"),
            ("spawn_", "sync"),
            ("spawn_", "command_line"),
            ("glib::", "spawn"),
            (".exec", "()"),
            ("libc::", "exec"),
            ("posix_", "spawn"),
            ("libc::", "system("),
            ("libc::", "fork("),
            ("po", "pen("),
        ]
        .iter()
        .map(|(a, b)| format!("{a}{b}"))
        .collect();
        let (begin, end) = (
            format!("// table{}begin", ':'),
            format!("// table{}end", ':'),
        );
        let (mut inside, mut tables, mut hits, mut builders) = (false, 0, Vec::new(), 0);
        for (n, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with(&begin) {
                inside = true;
                tables += 1;
                continue;
            }
            if trimmed.starts_with(&end) {
                inside = false;
                continue;
            }
            if inside {
                continue;
            }
            let code = line.split("//").next().unwrap_or("");
            if paths.iter().any(|p| line.contains(p.as_str()))
                || starts.iter().any(|p| code.contains(p.as_str()))
            {
                hits.push(format!("{}: {}", n + 1, line.trim()));
            }
            builders += code.matches(&format!("{command}::new(")).count();
        }
        assert_eq!((tables, inside), (1, false), "one closed table");
        assert!(
            hits.is_empty(),
            "absolute exec paths or program starts outside the table:\n{}",
            hits.join("\n")
        );
        assert_eq!(builders, 1, "one command builder (Setup::command)");
    }
}
