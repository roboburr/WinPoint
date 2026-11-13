use crate::{files::ApiError, i18n};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    os::unix::{
        fs::{MetadataExt, PermissionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(super) fn now() -> f64 {
    let mut t = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut t) };
    t.tv_sec as f64 + t.tv_nsec as f64 / 1e9
}
pub(super) fn wall() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
pub(super) fn ncpu() -> usize {
    unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) }.max(1) as usize
}
pub(super) fn clk() -> f64 {
    unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f64
}
pub(super) fn page() -> i64 {
    unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(1) as i64
}
pub(super) fn read(path: impl AsRef<Path>) -> String {
    fs::read(path)
        .map(|v| String::from_utf8_lossy(&v).trim().to_owned())
        .unwrap_or_default()
}
pub(super) fn read_bytes(path: impl AsRef<Path>, limit: u64) -> Option<Vec<u8>> {
    let f = fs::File::open(path).ok()?;
    let mut data = Vec::new();
    f.take(limit).read_to_end(&mut data).ok()?;
    Some(data)
}
pub(super) fn names(path: impl AsRef<Path>) -> Vec<String> {
    fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect()
}
pub(super) fn int(s: impl AsRef<str>) -> Option<i64> {
    s.as_ref().trim().parse().ok()
}
pub(super) fn num(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}
pub(super) fn valint(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().filter(|f| f.is_finite()).map(|f| f as i64)),
        Value::Bool(b) => Some(i64::from(*b)),
        Value::String(s) => int(s),
        _ => None,
    }
}
pub(super) fn truth(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|x| x != 0.),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}
pub(crate) fn round(v: f64, n: i32) -> f64 {
    if !v.is_finite() {
        return v;
    }
    format!("{:.*}", n.max(0) as usize, v).parse().unwrap_or(v)
}
/// `error` with its `data` (the refusals of restartPlan / restartProcess).
pub(super) fn with_data(mut error: ApiError, data: Value) -> ApiError {
    error.data = Some(data);
    error
}
pub(super) fn command_exists(name: &str) -> bool {
    which(name, None).is_some()
}
pub(super) fn which(name: &str, path: Option<&str>) -> Option<PathBuf> {
    if name.contains('/') {
        let p = PathBuf::from(name);
        return executable(&p).then_some(p);
    }
    let paths = path
        .map(str::to_owned)
        .unwrap_or_else(|| std::env::var("PATH").unwrap_or_else(|_| "/bin:/usr/bin".into()));
    paths
        .split(':')
        .map(|d| Path::new(if d.is_empty() { "." } else { d }).join(name))
        .find(|p| executable(p))
}
pub(super) fn executable(p: &Path) -> bool {
    fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
pub(super) struct Run {
    pub ok: bool,
    pub out: String,
    pub err: String,
}
pub(super) fn run(program: &str, args: &[&str], timeout: f64) -> Run {
    let mut c = Command::new(program);
    c.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        c.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut p = match c.spawn() {
        Ok(p) => p,
        Err(e) => {
            return Run {
                ok: false,
                out: String::new(),
                err: e.to_string(),
            };
        }
    };
    let stdout = p.stdout.take();
    let stderr = p.stderr.take();
    let o = std::thread::spawn(move || {
        let mut v = Vec::new();
        if let Some(s) = stdout {
            let _ = s.take(16 * 1024 * 1024).read_to_end(&mut v);
        }
        v
    });
    let e = std::thread::spawn(move || {
        let mut v = Vec::new();
        if let Some(s) = stderr {
            let _ = s.take(4 * 1024 * 1024).read_to_end(&mut v);
        }
        v
    });
    let start = Instant::now();
    let ok = loop {
        match p.try_wait() {
            Ok(Some(s)) => break s.success(),
            Ok(None) if start.elapsed() < Duration::from_secs_f64(timeout) => {
                std::thread::sleep(Duration::from_millis(10))
            }
            _ => {
                unsafe {
                    libc::kill(-(p.id() as i32), libc::SIGKILL);
                }
                let _ = p.wait();
                break false;
            }
        }
    };
    Run {
        ok,
        out: String::from_utf8_lossy(&o.join().unwrap_or_default()).into_owned(),
        err: String::from_utf8_lossy(&e.join().unwrap_or_default()).into_owned(),
    }
}
fn texts() -> &'static Value {
    static TEXT: OnceLock<Value> = OnceLock::new();
    TEXT.get_or_init(|| {
        serde_json::from_str(include_str!("text.json")).expect("static original sysmon strings")
    })
}
pub(super) fn tr(lang: &str, key: &str, args: &[(&str, String)]) -> String {
    let mut s = texts()["_STR"][lang][key]
        .as_str()
        .or_else(|| texts()["_STR"]["nl"][key].as_str())
        .unwrap_or(key)
        .to_owned();
    for (k, v) in args {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}
pub(super) fn friendly(lang: &str, names: &[&str]) -> Option<String> {
    for name in names {
        if let Some(a) = texts()["_FRIENDLY"][name].as_array() {
            return a
                .get(usize::from(lang == "en"))
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        if name.starts_with("gsd-") {
            return Some(i18n::tr(lang, "sysmon.unit.gnomeSettings"));
        }
    }
    None
}
/// The name of a text.json `{nl, en}` pair in `lang` (`_GSD`, `_GVFS`); `None` when the table has no `name`.
fn pair(lang: &str, table: &str, name: &str) -> Option<&'static str> {
    let names = &texts()[table][name];
    let nl = names["nl"].as_str()?;
    Some(i18n::pick(lang, nl, names["en"].as_str().unwrap_or(nl)))
}
/// systemd user units with a friendly name of their own (D-G9: rows in both languages); the key is rendered only
/// for the unit that is found.
const UNIT_NAMES: &[(&str, &str)] = &[
    ("speech-dispatcher.service", "sysmon.unit.speech"),
    ("at-spi-dbus-bus.service", "sysmon.unit.accessibility"),
    ("gnome-keyring-daemon.service", "sysmon.unit.keyring"),
    ("dconf.service", "sysmon.unit.settingsDatabase"),
    (
        "xdg-permission-store.service",
        "sysmon.unit.permissionStore",
    ),
    ("xdg-document-portal.service", "sysmon.unit.documentPortal"),
    ("xdg-desktop-portal.service", "sysmon.unit.desktopPortal"),
    ("gvfs-daemon.service", "sysmon.unit.fileSystemServices"),
    ("gvfs-metadata.service", "sysmon.unit.fileMetadata"),
    (
        "evolution-source-registry.service",
        "sysmon.unit.calendarContacts",
    ),
    ("evolution-calendar-factory.service", "sysmon.unit.calendar"),
    (
        "evolution-addressbook-factory.service",
        "sysmon.unit.addressBook",
    ),
    (
        "gnome-session-monitor.service",
        "sysmon.unit.sessionManager",
    ),
    ("mpris-proxy.service", "sysmon.unit.bluetoothMedia"),
    ("obex.service", "sysmon.unit.bluetoothTransfer"),
    ("filter-chain.service", "sysmon.unit.soundFilters"),
    ("flatpak-portal.service", "sysmon.unit.flatpak"),
    ("flatpak-session-helper.service", "sysmon.unit.flatpak"),
    ("gcr-ssh-agent.service", "sysmon.unit.sshKeys"),
    ("ssh-agent.service", "sysmon.unit.sshKeys"),
    ("localsearch-3.service", "sysmon.unit.fileIndexer"),
    ("tracker-miner-fs-3.service", "sysmon.unit.fileIndexer"),
    (
        "xdg-desktop-portal-gnome.service",
        "sysmon.unit.desktopPortalGnome",
    ),
    (
        "xdg-desktop-portal-gtk.service",
        "sysmon.unit.desktopPortalGtk",
    ),
    (
        "xdg-desktop-portal-hyprland.service",
        "sysmon.unit.desktopPortalHyprland",
    ),
    ("org.gnome.Evolution-alarm-notify", "sysmon.unit.reminders"),
    ("update-notifier", "sysmon.unit.updateNotifier"),
    ("snapd-desktop-integration", "sysmon.unit.snapIntegration"),
];
pub(super) fn unit_friendly(lang: &str, unit: &str) -> Option<String> {
    if let Some(part) = unit.strip_prefix("org.gnome.SettingsDaemon.") {
        let part = part.strip_suffix(".service").unwrap_or(part);
        return Some(match pair(lang, "_GSD", part) {
            Some(name) => i18n::trf(
                lang,
                "sysmon.unit.gnomeSettingsPart",
                &json!({"part": name}),
            ),
            None => i18n::tr(lang, "sysmon.unit.gnomeSettings"),
        });
    }
    if let Some(part) = unit
        .strip_prefix("gvfs-")
        .and_then(|s| s.strip_suffix("-volume-monitor.service"))
    {
        return Some(i18n::trf(
            lang,
            "sysmon.unit.volumeMonitor",
            &json!({"part": pair(lang, "_GVFS", part).unwrap_or(part)}),
        ));
    }
    if let Some((_, key)) = UNIT_NAMES.iter().find(|(k, _)| *k == unit) {
        return Some(i18n::tr(lang, key));
    }
    if unit.starts_with("gnome-session-manager@") {
        return Some(i18n::tr(lang, "sysmon.unit.sessionManager"));
    }
    if unit.starts_with("org.gnome.Shell@") {
        return Some("GNOME Shell".into());
    }
    if unit.starts_with("snap.") && unit.ends_with(".service") {
        return Some(i18n::trf(
            lang,
            "sysmon.unit.snap",
            &json!({"name": unit.split('.').nth(1).unwrap_or("")}),
        ));
    }
    None
}
pub(super) fn fmt_bytes(lang: &str, n: Option<f64>) -> String {
    let Some(n) = n else { return "–".into() };
    let mut v = n.abs();
    if v < 1024. {
        return format!("{} bytes", v.round_ties_even() as i64);
    }
    let units = ["KB", "MB", "GB", "TB", "PB"];
    let mut i = 0;
    v /= 1024.;
    while v >= 1024. && i < 4 {
        v /= 1024.;
        i += 1
    }
    let s = if v >= 100. {
        format!("{:.0}", round(v, 0))
    } else {
        format!("{:.1}", round(v, 1))
    };
    format!(
        "{}{} {}",
        if n < 0. { "-" } else { "" },
        if lang == "en" { s } else { s.replace('.', ",") },
        units[i]
    )
}
pub(super) fn fmt_duration(lang: &str, sec: f64) -> String {
    let sec = sec as i64;
    if sec < 60 {
        return i18n::trf(lang, "sysmon.duration.seconds", &json!({"sec": sec}));
    }
    let m = sec / 60;
    if m < 60 {
        return i18n::trn(lang, "sysmon.duration.minutes", m, i18n::NO_PARAMS);
    }
    format!(
        "{} {} {} min",
        m / 60,
        i18n::tr(lang, "sysmon.duration.hourUnit"),
        m % 60
    )
}
pub(super) fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if i + 3 < b.len() && b[i] == b'\\' && b[i + 1] == b'x' {
            if let Ok(v) = u8::from_str_radix(&s[i + 2..i + 4], 16) {
                out.push(char::from(v));
                i += 4;
                continue;
            }
        }
        let c = s[i..].chars().next().unwrap();
        out.push(c);
        i += c.len_utf8()
    }
    out
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Unit {
    pub kind: String,
    pub id: String,
    pub name: String,
}
const LAUNCHERS: &[&str] = &[
    "gnome", "flatpak", "kde", "hyprland", "Hyprland", "sway", "niri", "cosmic", "xdg", "uwsm",
    "dbus", "systemd", "gtk", "ubuntu",
];
pub(super) fn parse_cgroup(cgroup: &str) -> Option<Unit> {
    let last = cgroup.trim_end_matches('/').rsplit('/').next()?;
    let make = |kind: &str, id: String| {
        Some(Unit {
            kind: kind.into(),
            id: unescape(&id),
            name: last.into(),
        })
    };
    if let Some(base) = last
        .strip_prefix("app-")
        .and_then(|s| s.strip_suffix(".scope"))
    {
        let parts = base.split('-').collect::<Vec<_>>();
        let begin = usize::from(parts.len() >= 3 && LAUNCHERS.contains(&parts[0]));
        let end = if parts.len() >= 2 { parts.len() - 1 } else { 1 };
        return make("scope", parts[begin..end].join("-"));
    }
    if let Some(base) = last
        .strip_prefix("app-")
        .and_then(|s| s.strip_suffix(".service"))
    {
        let base = base.split('@').next().unwrap_or(base);
        let parts = base.split('-').collect::<Vec<_>>();
        return make(
            "service",
            if parts.len() >= 2 && LAUNCHERS.contains(&parts[0]) {
                parts[1..].join("-")
            } else {
                base.into()
            },
        );
    }
    if let Some(base) = last.strip_prefix("snap.") {
        for suffix in [".scope", ".service"] {
            if let Some(base) = base.strip_suffix(suffix) {
                if let Some((package, app)) = base.split_once('.') {
                    if !package.is_empty()
                        && package
                            .bytes()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
                    {
                        let app = app.split('.').next().unwrap_or(app);
                        let app = if let Some((a, b)) = app.split_once('-') {
                            if b.len() >= 29
                                && b.bytes().all(|c| c.is_ascii_hexdigit() || c == b'-')
                            {
                                a
                            } else {
                                app
                            }
                        } else {
                            app
                        };
                        if !app.is_empty()
                            && app.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
                        {
                            return make(
                                if suffix == ".scope" {
                                    "scope"
                                } else {
                                    "service"
                                },
                                format!("{package}_{app}"),
                            );
                        }
                    }
                }
            }
        }
    }
    if (last.ends_with(".service") || last.ends_with(".scope")) && cgroup.contains("/user@") {
        return make("unit", last.into());
    }
    None
}
pub(super) fn is_critical_comm(s: &str) -> bool {
    [
        "systemd",
        "Hyprland",
        "gnome-shell",
        "gnome-session-b",
        "gnome-session-c",
        "Xwayland",
        "dbus-daemon",
        "dbus-broker",
        "dbus-broker-lau",
        "gdm-wayland-ses",
        "gdm-x-session",
        "mutter-x11-fram",
        "sway",
        "kwin_wayland",
        "plasmashell",
        "uwsm",
        "start-hyprland",
        "(sd-pam)",
        "login",
        "systemd-logind",
        "Xorg",
    ]
    .contains(&s)
}
pub(super) fn critical_unit(s: &str) -> bool {
    if !(s.ends_with(".service") || s.ends_with(".target")) {
        return false;
    }
    let b = s.rsplit_once('.').map(|a| a.0).unwrap_or(s);
    [
        "dbus",
        "dbus-broker",
        "pipewire",
        "pipewire-pulse",
        "wireplumber",
        "at-spi-dbus-bus",
        "init",
    ]
    .contains(&b)
        || b.starts_with("org.gnome.Shell@")
        || b.starts_with("gnome-session")
        || b.starts_with("xdg-desktop-portal")
        || b.starts_with("wayland-wm@")
        || b.starts_with("graphical-session")
}
pub(super) fn interpreter(s: &str) -> bool {
    [
        "python",
        "python3",
        "node",
        "nodejs",
        "bash",
        "sh",
        "dash",
        "zsh",
        "fish",
        "perl",
        "ruby",
        "java",
        "env",
        "gjs",
        "lua",
        "php",
        "deno",
        "bun",
        "electron",
        "qemu-system-x86_64",
    ]
    .contains(&s)
        || s.strip_prefix("python3.")
            .is_some_and(|s| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit()))
}
pub(super) fn generic(s: &str) -> bool {
    interpreter(s)
        || [
            "flatpak",
            "snap",
            "bwrap",
            "sleep",
            "cat",
            "xdg-open",
            "sudo",
            "pkexec",
            "dbus-launch",
            "dbus-run-session",
            "systemd-run",
            "gio",
            "exo-open",
            "false",
            "true",
            "wine",
            "wine64",
            "steam",
            "env",
            "nohup",
            "setsid",
            "chrt",
            "nice",
        ]
        .contains(&s)
}
pub(super) fn basename(s: &str) -> &str {
    s.rsplit('/').next().unwrap_or(s)
}
pub(super) fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| glib::home_dir())
}
pub(super) fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
}
pub(super) fn list_units(state: Option<&str>) -> Vec<Value> {
    let mut args = vec!["--user", "list-units", "--type=service"];
    let state_arg;
    if let Some(s) = state {
        state_arg = format!("--state={s}");
        args.push(&state_arg)
    } else {
        args.push("--all")
    }
    args.push("--no-pager");
    let mut jsonargs = args.clone();
    jsonargs.push("--output=json");
    let r = run("systemctl", &jsonargs, 6.);
    if r.ok {
        if let Ok(Value::Array(a)) = serde_json::from_str(&r.out) {
            return a;
        }
    }
    args.extend(["--plain", "--no-legend"]);
    let r = run("systemctl", &args, 6.);
    if !r.ok {
        return vec![];
    }
    r.out.lines().filter_map(|l|{let mut f=l.split_whitespace();let unit=f.next()?;let load=f.next()?;let active=f.next()?;let sub=f.next()?;let description=f.collect::<Vec<_>>().join(" ");Some(json!({"unit":unit,"load":load,"active":active,"sub":sub,"description":description}))}).collect()
}
pub(super) fn unit_files() -> BTreeMap<String, String> {
    let r = run(
        "systemctl",
        &[
            "--user",
            "list-unit-files",
            "--type=service",
            "--no-pager",
            "--output=json",
        ],
        6.,
    );
    if !r.ok {
        return BTreeMap::new();
    }
    serde_json::from_str::<Vec<Value>>(&r.out)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| {
            Some((
                v["unit_file"].as_str()?.to_owned(),
                v["state"].as_str().unwrap_or("").to_owned(),
            ))
        })
        .collect()
}
pub(super) fn state_name(s: &str) -> &str {
    match s {
        "R" => "running",
        "S" => "sleeping",
        "D" | "W" => "waiting",
        "T" | "t" | "P" => "stopped",
        "Z" | "X" => "zombie",
        "I" => "idle",
        _ => "sleeping",
    }
}
pub(super) fn username(uid: u32) -> String {
    let mut pwd = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut result = std::ptr::null_mut();
    let mut buf = vec![0; 16384];
    let r = unsafe {
        libc::getpwuid_r(
            uid,
            pwd.as_mut_ptr(),
            buf.as_mut_ptr() as *mut _,
            buf.len(),
            &mut result,
        )
    };
    if r == 0 && !result.is_null() {
        let p = unsafe { pwd.assume_init() };
        return unsafe { std::ffi::CStr::from_ptr(p.pw_name) }
            .to_string_lossy()
            .into_owned();
    }
    uid.to_string()
}
pub(super) fn cgroup(pid: u32) -> String {
    let t = read(format!("/proc/{pid}/cgroup"));
    let l = t
        .lines()
        .find(|l| l.starts_with("0::"))
        .or_else(|| t.lines().next())
        .unwrap_or("");
    l.splitn(3, ':').nth(2).unwrap_or("").to_owned()
}
pub(super) fn metadata_uid(path: impl AsRef<Path>) -> Option<u32> {
    fs::metadata(path).ok().map(|m| m.uid())
}

#[cfg(test)]
mod rounding_tests {
    use super::*;
    #[test]
    fn decimal_rounding_matches_reference_binary_float_cases() {
        assert_eq!(round(2.675, 2), 2.67);
        assert_eq!(round(12.345, 2), 12.35);
        assert_eq!(round(2.5, 0), 2.);
        assert_eq!(round(3.5, 0), 4.);
    }
}
#[cfg(test)]
mod i18n_tests {
    use super::*;
    // D-G9: every unit friendly name is a catalogue row, and the GSD/GVFS part names have English halves
    #[test]
    fn every_unit_key_exists_and_every_part_name_has_english() {
        for (unit, key) in UNIT_NAMES {
            assert!(i18n::has(key), "{unit}: {key}");
        }
        for table in ["_GSD", "_GVFS"] {
            let names = texts()[table].as_object().unwrap();
            assert!(!names.is_empty());
            for (part, pair) in names {
                let (nl, en) = (pair["nl"].as_str().unwrap(), pair["en"].as_str().unwrap());
                assert!(
                    !en.is_empty() && en != nl,
                    "{table}.{part}: {nl:?} / {en:?}"
                );
                assert_eq!(pair.as_object().unwrap().len(), 2, "{table}.{part}");
            }
        }
    }
    /// 3b898e6's unit_friendly (its JSON tables read from their new place), for the comparison below.
    fn unit_friendly_3b898e6(lang: &str, unit: &str) -> Option<String> {
        if lang == "en" {
            return None;
        }
        if let Some(part) = unit.strip_prefix("org.gnome.SettingsDaemon.") {
            let part = part.strip_suffix(".service").unwrap_or(part);
            return Some(
                texts()["_GSD"][part]["nl"]
                    .as_str()
                    .map(|v| format!("GNOME-instellingen: {v}"))
                    .unwrap_or_else(|| "GNOME-instellingendienst".into()),
            );
        }
        if let Some(part) = unit
            .strip_prefix("gvfs-")
            .and_then(|s| s.strip_suffix("-volume-monitor.service"))
        {
            return Some(format!(
                "Stationsbewaking ({})",
                texts()["_GVFS"][part]["nl"].as_str().unwrap_or(part)
            ));
        }
        let table = [
            ("speech-dispatcher.service", "Spraakdienst"),
            ("at-spi-dbus-bus.service", "Toegankelijkheidsdienst"),
            ("gnome-keyring-daemon.service", "Sleutelbos (wachtwoorden)"),
            ("dconf.service", "Instellingendatabase"),
            ("xdg-permission-store.service", "Machtigingenopslag"),
            ("xdg-document-portal.service", "Documentportaal"),
            ("xdg-desktop-portal.service", "Bureaubladportaal"),
            ("gvfs-daemon.service", "Bestandssysteemdiensten (GVfs)"),
            ("gvfs-metadata.service", "Bestandsgegevens"),
            ("evolution-source-registry.service", "Agenda en contacten"),
            ("evolution-calendar-factory.service", "Agenda-dienst"),
            ("evolution-addressbook-factory.service", "Adresboekdienst"),
            ("gnome-session-monitor.service", "GNOME-sessiebeheer"),
            ("mpris-proxy.service", "Bluetooth-mediabediening"),
            ("obex.service", "Bluetooth-bestandsoverdracht"),
            ("filter-chain.service", "Geluidsfilters (PipeWire)"),
            ("flatpak-portal.service", "Flatpak-dienst"),
            ("flatpak-session-helper.service", "Flatpak-dienst"),
            ("gcr-ssh-agent.service", "SSH-sleutels"),
            ("ssh-agent.service", "SSH-sleutels"),
            ("localsearch-3.service", "Bestanden indexeren"),
            ("tracker-miner-fs-3.service", "Bestanden indexeren"),
            (
                "xdg-desktop-portal-gnome.service",
                "Bureaubladportaal (GNOME)",
            ),
            ("xdg-desktop-portal-gtk.service", "Bureaubladportaal (GTK)"),
            (
                "xdg-desktop-portal-hyprland.service",
                "Bureaubladportaal (Hyprland)",
            ),
            ("org.gnome.Evolution-alarm-notify", "Herinneringen"),
            ("update-notifier", "Updatemeldingen"),
            ("snapd-desktop-integration", "Snap-integratie"),
        ];
        if let Some((_, v)) = table.iter().find(|(k, _)| *k == unit) {
            return Some((*v).into());
        }
        if unit.starts_with("gnome-session-manager@") {
            return Some("GNOME-sessiebeheer".into());
        }
        if unit.starts_with("org.gnome.Shell@") {
            return Some("GNOME Shell".into());
        }
        if unit.starts_with("snap.") && unit.ends_with(".service") {
            return Some(format!(
                "Snap-dienst ({})",
                unit.split('.').nth(1).unwrap_or("")
            ));
        }
        None
    }
    /// Dutch is byte-identical for every unit the old code named, and the per-row cost of the process list
    /// (unit_friendly runs once per unit process in sysmon.processes) did not grow (F28). Printed with --nocapture.
    #[test]
    fn unit_friendly_matches_3b898e6_in_dutch_and_costs_no_more() {
        let mut units: Vec<String> = UNIT_NAMES.iter().map(|(u, _)| u.to_string()).collect();
        for part in texts()["_GSD"].as_object().unwrap().keys() {
            units.push(format!("org.gnome.SettingsDaemon.{part}.service"));
        }
        for part in texts()["_GVFS"].as_object().unwrap().keys() {
            units.push(format!("gvfs-{part}-volume-monitor.service"));
        }
        units.extend(
            [
                "org.gnome.SettingsDaemon.Other.service",
                "gvfs-other-volume-monitor.service",
                "gnome-session-manager@ubuntu.service",
                "org.gnome.Shell@wayland.service",
                "snap.firefox.firefox.service",
                "pipewire.service",
                "app-gnome-firefox-1234.scope",
            ]
            .map(String::from),
        );
        for unit in &units {
            assert_eq!(
                unit_friendly("nl", unit),
                unit_friendly_3b898e6("nl", unit),
                "{unit}"
            );
        }
        let time = |f: &dyn Fn(&str, &str) -> Option<String>, lang: &str| {
            let start = std::time::Instant::now();
            let mut n = 0;
            for _ in 0..200 {
                for unit in &units {
                    n += f(lang, unit).map(|s| s.len()).unwrap_or(0);
                }
            }
            assert!(n > 0 || lang == "en");
            start.elapsed().as_secs_f64() * 1e6 / (200 * units.len()) as f64
        };
        let (old_nl, new_nl) = (
            time(&unit_friendly_3b898e6, "nl"),
            time(&unit_friendly, "nl"),
        );
        let (old_en, new_en) = (
            time(&unit_friendly_3b898e6, "en"),
            time(&unit_friendly, "en"),
        );
        eprintln!(
            "unit_friendly per call ({} units): nl 3b898e6 {old_nl:.3} µs, now {new_nl:.3} µs; en 3b898e6 {old_en:.3} µs (returned None), now {new_en:.3} µs",
            units.len()
        );
        // a loose bound (debug build, shared machine): a real regression would be far above it
        assert!(new_nl < old_nl * 4. + 2., "nl {new_nl} µs vs {old_nl} µs");
    }
}
