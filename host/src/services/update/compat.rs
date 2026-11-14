//! The compatibility check of an offered Ubuntu release (design §6.2, C13/C16 cheap verdict first, C17/D-G27), GW6.
//!
//! * **Cheap verdict first** ([`cheap`]): `releases.json` (embedded; a fixture root may hold its own) and the
//!   *installed* extension's `shell-version` (`$XDG_DATA_HOME/gnome-shell/extensions/<uuid>/metadata.json`). A
//!   codename whose `status` is not `tested`, or whose `gnomeShell` major no `shell-version` entry matches (GNOME's
//!   prefix rule: `"50"` or `"50.x"`), is `hold`, and nothing is built or downloaded.
//! * **The full check** ([`run_full`], only for a tested codename): our binaries' NEEDED sonames ([`super::elf`];
//!   libc, libgcc_s, ld-linux skipped) plus `libarchive.so.13` (loaded at run time) → their packages through
//!   `dpkg-query -S` (the tool table); a private apt tree `<cache>/update/release-check/<codename>/` with its own
//!   `APT_CONFIG` ([`apt_conf`]: every `Dir::Etc*` inside the tree, an empty `apt.conf.d`, `Debug::NoLocking`, the
//!   system's dpkg status read only, `Acquire::http::Proxy` copied from the system's `apt-config dump`); its sources
//!   are the Ubuntu stanzas of `ubuntu.sources` with the codename's suites (`<c>`, `<c>-updates`, `<c>-security`),
//!   their own URIs and components, and `Signed-By` forced to Ubuntu's archive keyring (GW6's throwaway key in test
//!   mode). `apt-config dump` under the tree must show no hook and no `Dir::Etc*` outside it ([`config_problems`]),
//!   then `apt-get -q update`, `apt-cache policy`, `apt-get -s -q dist-upgrade` (nice 10, idle I/O, `LANG=C`), parsed
//!   by [`parse_policy`] and [`parse_simulation`]; the tree is deleted afterwards, whatever happened.
//! * **Verdict** ([`evaluate`], [`decide`]): `blocked` (a blocking package has no version in the new archive or
//!   the simulation removes it, unless a renamed package of the same library is installed instead) → `hold` (the
//!   archive's gnome-shell major is not in `shell-version`) → `ubuntuHold` (meta-release `UpgradeBroken`, applied
//!   when the snapshot is read, so it follows Ubuntu's file) → `warn` (a degraded package goes, or third-party
//!   sources exist) → `ok`; `unknown` with a reason when the check is not possible (offline, metered without
//!   `update.metered`, an apt failure) or failed.
//! * **Cache** `compat-<codename>.json` in the update cache: the verdict, its reasons as keys and params (rendered per
//!   call), the archive's gnome-shell major, the archive's Release date, `checkedAt` and the inputs it was made from
//!   (extension version, `shell-version`, the `releases.json` entry, the offered version); valid for 7 days (a
//!   failed check for 1 h) while the inputs stay the same.
//! * **`update.release.compat {refresh?}`** ([`compat`]): a write method (never in dry-run, forwarded by a mirror,
//!   refused while the upgrader runs; `update.rs` does that). It starts the full check on its own thread and answers
//!   `{started:true}`, or `{started:false, skipped}` (`notOffered`, `hold`, `recent`, `offline`, `metered`,
//!   `running`); `refresh` only re-runs a failed check. `update.changed` goes out when it starts and when it ends.
//!   The page asks for it when the snapshot says `state:'checking'` with `compat.needed` (an offered, tested
//!   release without a verdict), and from the card's Retry.
//! * [`fill`] adds the card's fields to the snapshot's `release` object; [`cached`] is the verdict known without
//!   running anything (GW5's `update.release.ubuntuHold` needs `hold`/`blocked`).
use super::{
    Shared, Stats, elf, net,
    paths::{Paths, Setup, Tool},
    release::Offer,
    store,
};
use crate::{files::ApiResult, i18n::Lang};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    io::{self, Read},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

const RELEASES: &str = include_str!("../../../data/releases.json");
/// A verdict counts for 7 days (design §6.2: at most once per release per 7 days); a failed check for 1 hour.
const VALID_MS: u64 = 7 * 24 * 3600 * 1000;
const RETRY_MS: u64 = 3600 * 1000;
/// Held while a check runs (one per user; readers see it as `compat.running`).
const LOCK: &str = "compat.lock";
const TREE: &str = "release-check";
const UPDATE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const QUERY_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const MAX_OUTPUT: u64 = 8 << 20;
/// libarchive is loaded at run time (README: "libarchive13t64"), so no NEEDED names it.
const ARCHIVE_SONAME: &str = "libarchive.so.13";
const ARCHIVE_PACKAGE: &str = "libarchive13t64";
const UPGRADER_MO: &str = "nl/LC_MESSAGES/ubuntu-release-upgrader.mo";
/// The degraded parts (warn): part code → the packages of which one must stay (design §6.2).
const DEGRADED: &[(&str, &[&str])] = &[
    ("ffmpeg", &["ffmpeg"]),
    ("bubblewrap", &["bubblewrap"]),
    ("squashfs", &["squashfs-tools"]),
    ("flatpak", &["flatpak"]),
    ("packagekit", &["packagekit"]),
    ("terminal", &["ptyxis", "gnome-terminal", "gnome-console"]),
    ("terminal", &["xdg-terminal-exec"]),
    ("screen", &["screen"]),
    ("dock", &["gnome-shell-ubuntu-extensions"]),
];

// ------------------------------------------------------------------------------------------------ inputs

/// `releases.json`: the fixture root's own copy in test mode when it exists, else the embedded one.
pub(crate) fn releases(paths: &Paths) -> Value {
    static EMBEDDED: OnceLock<Value> = OnceLock::new();
    if let Some(text) = paths
        .releases
        .as_ref()
        .and_then(|p| fs::read_to_string(p).ok())
    {
        return serde_json::from_str(&text).unwrap_or(Value::Null);
    }
    EMBEDDED
        .get_or_init(|| serde_json::from_str(RELEASES).unwrap_or(Value::Null))
        .clone()
}
/// The codename's entry (`{version, gnomeShell, status, allowAnyway?}`), or Null.
pub(crate) fn entry(releases: &Value, codename: &str) -> Value {
    releases["releases"][codename].clone()
}

/// The installed extension: its `version` (number or string) and `shell-version`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Extension {
    pub version: Option<String>,
    pub shell: Vec<String>,
}
/// The installed extension's metadata.json (an empty Extension when it is missing or unreadable: every major then
/// fails the match, so the verdict holds).
pub(crate) fn extension(paths: &Paths, releases: &Value) -> Extension {
    let uuid = releases["extension"].as_str().unwrap_or("");
    if uuid.is_empty() || uuid.contains('/') || uuid.starts_with('.') {
        return Extension::default();
    }
    let path = paths
        .user_data
        .join("gnome-shell/extensions")
        .join(uuid)
        .join("metadata.json");
    let meta: Value = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null);
    let version = match &meta["version"] {
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) => Some(s.clone()),
        _ => None,
    };
    let shell = meta["shell-version"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Extension { version, shell }
}
/// GNOME's rule (`extensionSystem.js`): an entry equal to the major, or the major followed by a dot.
pub(crate) fn shell_matches(shell: &[String], major: &str) -> bool {
    !major.is_empty()
        && shell.iter().any(|v| {
            v == major
                || v.strip_prefix(major)
                    .is_some_and(|rest| rest.starts_with('.'))
        })
}
/// The digits before the first dot, after an epoch: "51.0-0ubuntu1" → "51", "1:2.3" → "2".
pub(crate) fn major(version: &str) -> Option<String> {
    let version = version.split_once(':').map_or(version, |(_, rest)| rest);
    let digits: String = version.chars().take_while(char::is_ascii_digit).collect();
    (!digits.is_empty()).then_some(digits)
}

/// The cheap verdict (C13/C16): hold, or the full check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cheap {
    Hold,
    Full,
}
pub(crate) fn cheap(entry: &Value, extension: &Extension) -> Cheap {
    let tested = entry["status"] == "tested";
    let matches = entry["gnomeShell"]
        .as_str()
        .is_some_and(|m| shell_matches(&extension.shell, m));
    if tested && matches {
        Cheap::Full
    } else {
        Cheap::Hold
    }
}

/// What a verdict was made from: a different one makes the cache invalid.
fn inputs(entry: &Value, extension: &Extension, offer: &Offer) -> Value {
    json!({"extensionVersion":extension.version,"shellVersion":extension.shell,"entry":entry,
           "version":offer.version})
}

// ------------------------------------------------------------------------------------------------ sources

/// The paragraphs of a deb822 file (keys lowercased; continuation lines joined with a newline).
pub(crate) fn deb822(text: &str) -> Vec<BTreeMap<String, String>> {
    let mut out = Vec::new();
    let mut current: BTreeMap<String, String> = BTreeMap::new();
    let mut last: Option<String> = None;
    for line in text.lines() {
        if line.starts_with('#') {
            continue;
        }
        if line.trim().is_empty() {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            last = None;
            continue;
        }
        if line.starts_with([' ', '\t']) {
            if let Some(value) = last.as_ref().and_then(|k| current.get_mut(k)) {
                value.push('\n');
                value.push_str(line.trim());
            }
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim().to_ascii_lowercase();
            current.insert(key.clone(), value.trim().to_owned());
            last = Some(key);
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}
fn words(value: Option<&String>) -> Vec<String> {
    value
        .map(|v| v.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default()
}
fn enabled(p: &BTreeMap<String, String>) -> bool {
    words(p.get("types")).iter().any(|t| t == "deb")
        && !matches!(
            p.get("enabled").map(|v| v.to_ascii_lowercase()).as_deref(),
            Some("no" | "false")
        )
}
/// A URI the tree may use: http(s) always, `file:` only in test mode; nothing that could break a deb822 field.
fn usable_uri(uri: &str, allow_file: bool) -> bool {
    let ok_chars = uri
        .bytes()
        .all(|b| b.is_ascii_graphic() && b != b'"' && b != b'#');
    ok_chars
        && (uri.starts_with("http://")
            || uri.starts_with("https://")
            || (allow_file && uri.starts_with("file:/")))
}
/// One stanza of the tree's sources.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Stanza {
    pub uris: Vec<String>,
    pub suites: Vec<String>,
    pub components: Vec<String>,
}
/// The Ubuntu stanzas of `ubuntu.sources` (enabled `deb` stanzas naming the current codename's suites) with the new
/// codename's suites (`<c>`, `<c>-updates`, `<c>-security`; backports and proposed are left out), their own URIs and
/// components (design §6.2: components from the file, never forced).
pub(crate) fn target_stanzas(
    text: &str,
    current: &str,
    codename: &str,
    allow_file: bool,
) -> Vec<Stanza> {
    deb822(text)
        .iter()
        .filter(|p| enabled(p))
        .filter_map(|p| {
            let suites: Vec<String> = words(p.get("suites"))
                .iter()
                .filter_map(|s| match s.strip_prefix(current) {
                    Some("") => Some(codename.to_owned()),
                    Some(rest @ ("-updates" | "-security")) => Some(format!("{codename}{rest}")),
                    _ => None,
                })
                .collect();
            let uris = words(p.get("uris"));
            let components = words(p.get("components"));
            let usable = !uris.is_empty() && uris.iter().all(|u| usable_uri(u, allow_file));
            let parts_ok = components
                .iter()
                .all(|c| c.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
            (!suites.is_empty() && usable && !components.is_empty() && parts_ok).then_some(Stanza {
                uris,
                suites,
                components,
            })
        })
        .collect()
}
fn host_of(uri: &str) -> Option<String> {
    let rest = uri.split_once("://")?.1;
    let host = rest.split(['/', ':']).next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}
fn ubuntu_host(host: &str) -> bool {
    host == "ubuntu.com" || host.ends_with(".ubuntu.com")
}
/// Third-party sources (design §6.2: `/etc/apt/sources.list.d` minus `ubuntu.sources`): the `.list`/`.sources`
/// files with an active non-Ubuntu `deb` entry, and the host (or file name) of the first one, in file-name order.
pub(crate) fn third_party(dir: &Path) -> (usize, Option<String>) {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    files.sort();
    let mut count = 0;
    let mut first = None;
    for path in files {
        let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            continue;
        };
        if name == "ubuntu.sources" || name.starts_with('.') {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let uris: Vec<String> = if name.ends_with(".sources") {
            deb822(&text)
                .iter()
                .filter(|p| enabled(p))
                .flat_map(|p| words(p.get("uris")))
                .collect()
        } else if name.ends_with(".list") {
            text.lines()
                .map(str::trim)
                .filter(|l| l.split_whitespace().next() == Some("deb"))
                .filter_map(|l| {
                    let mut fields = l.split_whitespace().skip(1);
                    let mut uri = fields.next()?;
                    if uri.starts_with('[') {
                        // options: "[arch=amd64 signed-by=…]" up to the closing bracket
                        while !uri.ends_with(']') {
                            uri = fields.next()?;
                        }
                        uri = fields.next()?;
                    }
                    Some(uri.to_owned())
                })
                .collect()
        } else {
            continue;
        };
        let foreign: Vec<&String> = uris
            .iter()
            .filter(|u| !host_of(u).is_some_and(|h| ubuntu_host(&h)))
            .collect();
        if let Some(uri) = foreign.first() {
            count += 1;
            if first.is_none() {
                let stem = name
                    .rsplit_once('.')
                    .map_or(name.as_str(), |(s, _)| s)
                    .to_owned();
                first = Some(host_of(uri).unwrap_or(stem));
            }
        }
    }
    (count, first)
}

// ------------------------------------------------------------------------------------------------ the tree

fn quoted(path: &Path) -> io::Result<String> {
    let text = path
        .to_str()
        .ok_or_else(|| io::Error::other("tree path is not UTF-8"))?;
    if text.contains(['"', '\n', '\r', ';', '{', '}']) {
        return Err(io::Error::other("tree path cannot be quoted"));
    }
    Ok(text.to_owned())
}
/// A proxy value apt.conf may carry as is (no quote or separator inside).
fn proxy_ok(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+-._~:/@%[]!$&'()*,=".contains(&b))
}
/// The tree's own `APT_CONFIG` (design §6.2): every `Dir::Etc*` and every state and cache directory inside the tree,
/// the system's dpkg status (re-rooted in test mode) read only, no lock, one architecture, no translations and no
/// extra index targets.
pub(crate) fn apt_conf(tree: &Path, paths: &Paths, proxy: Option<&str>) -> io::Result<String> {
    let t = quoted(tree)?;
    let status = quoted(&paths.dpkg_status)?;
    let extended = quoted(&paths.apt_extended_states)?;
    let mut conf = format!(
        "Dir \"{t}/\";\nDir::Etc \"{t}/etc/apt/\";\nDir::Etc::Main \"{t}/etc/apt/apt.conf\";\n\
         Dir::Etc::Parts \"{t}/etc/apt/apt.conf.d/\";\nDir::Etc::SourceList \"{t}/etc/apt/sources.list\";\n\
         Dir::Etc::SourceParts \"{t}/etc/apt/sources.list.d/\";\nDir::Etc::Preferences \"{t}/etc/apt/preferences\";\n\
         Dir::Etc::PreferencesParts \"{t}/etc/apt/preferences.d/\";\nDir::Etc::Trusted \"{t}/etc/apt/trusted.gpg\";\n\
         Dir::Etc::TrustedParts \"{t}/etc/apt/trusted.gpg.d/\";\nDir::State \"{t}/var/lib/apt/\";\n\
         Dir::State::Lists \"{t}/var/lib/apt/lists/\";\nDir::State::status \"{status}\";\n\
         Dir::State::extended_states \"{extended}\";\nDir::Cache \"{t}/var/cache/apt/\";\nDir::Log \"{t}/var/log/apt/\";\n\
         Debug::NoLocking \"1\";\nAPT::Architectures {{ \"amd64\"; }};\nAcquire::Languages \"none\";\n\
         Acquire::IndexTargets::deb::Contents-deb::DefaultEnabled \"false\";\n\
         Acquire::IndexTargets::deb::DEP-11::DefaultEnabled \"false\";\n\
         Acquire::IndexTargets::deb::CNF::DefaultEnabled \"false\";\n"
    );
    if let Some(proxy) = proxy.filter(|p| proxy_ok(p)) {
        conf.push_str(&format!("Acquire::http::Proxy \"{proxy}\";\n"));
    }
    Ok(conf)
}
/// The tree's sources (deb822), `Signed-By` forced to the archive keyring.
pub(crate) fn sources(stanzas: &[Stanza], keyring: &Path) -> io::Result<String> {
    let key = quoted(keyring)?;
    Ok(stanzas
        .iter()
        .map(|s| {
            // deb822 fields (machine text for apt, never shown)
            let fields = [
                ("Types", "deb".to_owned()),
                ("URIs", s.uris.join(" ")),
                ("Suites", s.suites.join(" ")),
                ("Components", s.components.join(" ")),
                ("Signed-By", key.clone()),
            ];
            fields
                .iter()
                .map(|(k, v)| format!("{k}: {v}\n"))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n"))
}
/// `Key "value";` lines of `apt-config dump`.
pub(crate) fn parse_config_dump(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let (key, rest) = l.split_once(' ')?;
            let value = rest
                .strip_suffix(';')?
                .trim()
                .strip_prefix('"')?
                .strip_suffix('"')?;
            Some((key.to_owned(), value.to_owned()))
        })
        .collect()
}
/// `Acquire::http::Proxy` of the system's configuration (copied into the tree, design §6.2).
pub(crate) fn proxy(dump: &[(String, String)]) -> Option<String> {
    dump.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("Acquire::http::Proxy"))
        .map(|(_, v)| v.clone())
        .filter(|v| proxy_ok(v))
}
/// What `apt-config dump` under the tree must not show (C17/D-G27): a hook (any `…Invoke…`, `…Hooks…`,
/// `Pre-Install-Pkgs`), or a configuration, source, preference, list or cache directory outside the tree.
pub(crate) fn config_problems(dump: &[(String, String)], tree: &Path) -> Vec<String> {
    let inside = |v: &str| Path::new(v).starts_with(tree);
    let mut problems: Vec<String> = dump
        .iter()
        .filter(|(k, _)| {
            let k = k.to_ascii_lowercase();
            k.contains("invoke") || k.contains("::hooks") || k.contains("pre-install-pkgs")
        })
        .map(|(k, v)| format!("hook {k} {v}"))
        .collect();
    let must_be_inside = [
        "Dir::Etc",
        "Dir::Etc::Main",
        "Dir::Etc::Parts",
        "Dir::Etc::SourceList",
        "Dir::Etc::SourceParts",
        "Dir::Etc::Preferences",
        "Dir::Etc::PreferencesParts",
        "Dir::Etc::TrustedParts",
        "Dir::State::Lists",
        "Dir::Cache",
    ];
    for key in must_be_inside {
        match dump.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
            Some((_, v)) if inside(v) => {}
            Some((k, v)) => problems.push(format!("outside the tree: {k} {v}")),
            None => problems.push(format!("not set: {key}")),
        }
    }
    problems
}

/// The tree on disk; deleted when dropped (design §6.2: "the tree is deleted").
struct Tree(PathBuf);
impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn tree_dir(shared: &Shared, codename: &str) -> PathBuf {
    shared
        .ctx
        .cache_dir
        .join("update")
        .join(TREE)
        .join(codename)
}
fn build_tree(dir: &Path, conf: &str, list: &str) -> io::Result<Tree> {
    let _ = fs::remove_dir_all(dir);
    let tree = Tree(dir.to_path_buf());
    for sub in [
        "etc/apt/apt.conf.d",
        "etc/apt/sources.list.d",
        "etc/apt/preferences.d",
        "etc/apt/trusted.gpg.d",
        "var/lib/apt/lists/partial",
        "var/cache/apt/archives/partial",
        "var/log/apt",
    ] {
        store::private_dir(&dir.join(sub))?;
    }
    fs::write(dir.join("etc/apt/apt.conf"), conf)?;
    fs::write(dir.join("etc/apt/sources.list.d/release.sources"), list)?;
    Ok(tree)
}

// ------------------------------------------------------------------------------------------------ programs

/// One finished program: exit code (None: killed after its timeout) and its output.
#[derive(Debug, Default)]
pub(crate) struct Out {
    pub rc: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}
/// A program of the tool table, nice 10 and idle I/O (in the child: no extra program), `LANG=C`, stdin null.
fn run(
    setup: &Setup,
    tool: Tool,
    args: &[OsString],
    env: &[(&str, OsString)],
    timeout: Duration,
) -> Option<Out> {
    let mut command = setup.tools.command(tool).ok()?;
    command.args(args).env("LANG", "C").env("LC_ALL", "C");
    for (key, value) in env {
        command.env(key, value);
    }
    if !setup.test {
        for key in [
            "http_proxy",
            "https_proxy",
            "no_proxy",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "NO_PROXY",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(|| {
            libc::nice(10);
            // ioprio_set(IOPRIO_WHO_PROCESS, self, IOPRIO_CLASS_IDLE << 13): ionice -c3
            libc::syscall(libc::SYS_ioprio_set, 1, 0, 3 << 13);
            Ok(())
        });
    }
    let mut child = command.spawn().ok()?;
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(pipe) = pipe {
                let _ = pipe.take(MAX_OUTPUT).read_to_end(&mut bytes);
            }
            String::from_utf8_lossy(&bytes).into_owned()
        })
    };
    let out = drain(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let err = drain(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let started = Instant::now();
    let rc = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(50))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    Some(Out {
        rc,
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
    })
}

// ------------------------------------------------------------------------------------------------ parsers

/// A package name apt and dpkg accept (and nothing a program could read as an option).
pub(crate) fn package_name(name: &str) -> bool {
    name.len() >= 2
        && name.len() <= 128
        && name
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"+-.".contains(&b))
}
fn strip_arch(name: &str) -> &str {
    name.split_once(':').map_or(name, |(n, _)| n)
}
/// `dpkg-query -S <paths>` → soname (the path's file name) → its packages. Diversion lines are skipped.
pub(crate) fn parse_dpkg_search(text: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for line in text.lines().filter(|l| !l.starts_with("diversion ")) {
        let Some((packages, path)) = line.rsplit_once(": /") else {
            continue;
        };
        let Some(soname) = path.rsplit('/').next() else {
            continue;
        };
        for package in packages
            .split(", ")
            .map(|p| strip_arch(p.trim()))
            .filter(|p| package_name(p))
        {
            map.entry(soname.to_owned())
                .or_default()
                .insert(package.to_owned());
        }
    }
    map
}
/// One package of `apt-cache policy`: installed and candidate versions, and the versions from an archive (a source
/// line that is not a local file such as the dpkg status).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Policy {
    pub installed: Option<String>,
    pub candidate: Option<String>,
    pub archive: Vec<String>,
}
pub(crate) fn parse_policy(text: &str) -> BTreeMap<String, Policy> {
    let mut map: BTreeMap<String, Policy> = BTreeMap::new();
    let mut name: Option<String> = None;
    let mut version: Option<String> = None;
    for line in text.lines() {
        if !line.starts_with(' ') {
            name = line.strip_suffix(':').map(|n| strip_arch(n).to_owned());
            version = None;
            if let Some(n) = &name {
                map.entry(n.clone()).or_default();
            }
            continue;
        }
        let Some(entry) = name.as_ref().and_then(|n| map.get_mut(n)) else {
            continue;
        };
        let trimmed = line.trim();
        let none = |v: &str| (v != "(none)").then(|| v.to_owned());
        if let Some(v) = trimmed.strip_prefix("Installed: ") {
            entry.installed = none(v.trim());
        } else if let Some(v) = trimmed.strip_prefix("Candidate: ") {
            entry.candidate = none(v.trim());
        } else if trimmed == "Version table:" {
        } else if line.len() - line.trim_start().len() >= 7 {
            // a source line of the current version: "500 http://… stonking/main amd64 Packages" or "100 /var/lib/dpkg/status"
            let source = trimmed.split_whitespace().nth(1).unwrap_or("");
            if let Some(v) = &version
                && !source.starts_with('/')
                && !entry.archive.contains(v)
            {
                entry.archive.push(v.clone());
            }
        } else {
            // a version line: "     51.0-0ubuntu1 500" or " *** 50.1-0ubuntu1.5 100"
            version = trimmed
                .trim_start_matches("*** ")
                .split_whitespace()
                .next()
                .map(str::to_owned);
        }
    }
    map
}
/// One line of the dist-upgrade simulation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Sim {
    /// `Inst <pkg> [<old>] (<new> <origin> [<arch>])`
    Inst { name: String, new: String },
    /// `Remv <pkg> [<old>]` (and `Purg`)
    Remv { name: String },
}
/// The simulation's `Inst`/`Remv` lines; an `E:` line is an error (apt could not simulate the upgrade).
pub(crate) fn parse_simulation(text: &str) -> Result<Vec<Sim>, String> {
    if let Some(error) = text.lines().find(|l| l.starts_with("E: ")) {
        return Err(error.to_owned());
    }
    Ok(text
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let kind = fields.next()?;
            let name = strip_arch(fields.next()?).to_owned();
            match kind {
                "Inst" => {
                    let new = line
                        .split_once(" (")?
                        .1
                        .split_whitespace()
                        .next()?
                        .to_owned();
                    Some(Sim::Inst { name, new })
                }
                "Remv" | "Purg" => Some(Sim::Remv { name }),
                _ => None,
            }
        })
        .collect())
}
/// The `Date:` of the codename's Release file in the tree's lists (the archive's date for the cache).
fn archive_date(lists: &Path, codename: &str) -> Option<String> {
    let suffixes = [
        format!("_dists_{codename}_InRelease"),
        format!("_dists_{codename}_Release"),
    ];
    let file = fs::read_dir(lists)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| suffixes.iter().any(|s| n.ends_with(s)))
        })?;
    let text = fs::read_to_string(file).ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix("Date: "))
        .map(|d| d.trim().to_owned())
}

// ------------------------------------------------------------------------------------------------ verdict

/// The part text of a package (an `update.part.*` key, else the package name itself).
pub(crate) fn part_of(package: &str) -> String {
    let code = match package {
        "gnome-shell" => Some("gnomeShell"),
        p if p.starts_with("libwebkit2gtk-4.1-") => Some("webkit"),
        p if p.starts_with("libjavascriptcoregtk-4.1-") => Some("jsc"),
        p if p.starts_with("libgtk-3-") => Some("gtk"),
        p if p.starts_with("libgtk-4-")
            || p.starts_with("libgraphene-1.0-")
            || p.starts_with("libpango-1.0-") =>
        {
            Some("gtk4")
        }
        p if p.starts_with("libsoup-3.0-") => Some("soup"),
        p if p.starts_with("libglib2.0-") => Some("glib"),
        p if p.starts_with("libgdk-pixbuf-2.0-") => Some("pixbuf"),
        p if p.starts_with("libcairo2") => Some("cairo"),
        p if p.starts_with("zlib1g") => Some("zlib"),
        p if p.starts_with("libarchive1") => Some("archive"),
        _ => None,
    };
    code.map_or_else(|| package.to_owned(), |c| format!("update.part.{c}"))
}
/// A library package without its ABI suffix (t64 and the like): a renamed package of the same library has the
/// same stem ("libgtk-3-0t64" → "libgtk-3-0").
fn stem(name: &str) -> &str {
    let trimmed = name.trim_end_matches(|c: char| c.is_ascii_digit());
    match trimmed.strip_suffix(['t', 'v']) {
        Some(base)
            if trimmed.len() < name.len() && base.ends_with(|c: char| c.is_ascii_digit()) =>
        {
            base
        }
        _ => name,
    }
}
fn reason(code: &str, key: &str, params: Value) -> Value {
    json!({"code":code,"key":key,"params":params})
}
/// What the full check found, for [`evaluate`].
#[derive(Clone, Debug, Default)]
pub(crate) struct Facts {
    /// installed packages owning our NEEDED sonames, plus libarchive
    pub blocking: BTreeSet<String>,
    pub policy: BTreeMap<String, Policy>,
    pub sim: Vec<Sim>,
    pub third_party: (usize, Option<String>),
    pub shell: Vec<String>,
    /// `releases.json`'s `gnomeShell` (a difference to the archive is a note for us, never a reason)
    pub expected_major: Option<String>,
    /// the offered version (`{version}` of the reasons)
    pub version: String,
}
/// The archive verdict `{state, reasons, gnomeMajor, notes}`: blocked → hold → warn → ok (ubuntuHold is applied when
/// the snapshot is read, [`decide`]).
pub(crate) fn evaluate(f: &Facts) -> Value {
    let installs: BTreeMap<&str, &str> = f
        .sim
        .iter()
        .filter_map(|s| match s {
            Sim::Inst { name, new } => Some((name.as_str(), new.as_str())),
            _ => None,
        })
        .collect();
    let removed: BTreeSet<&str> = f
        .sim
        .iter()
        .filter_map(|s| match s {
            Sim::Remv { name } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    let present = |p: &str| f.policy.get(p).is_some_and(|p| !p.archive.is_empty());
    // a renamed package of the same library that the upgrade installs instead (t64-style)
    let replaced = |p: &str| {
        installs
            .keys()
            .any(|i| *i != p && stem(i) == stem(p) && !removed.contains(i))
    };
    let version = f.version.as_str();
    let mut blocking = Vec::new();
    for package in f.blocking.iter().map(String::as_str).chain(["gnome-shell"]) {
        let params = json!({"part":part_of(package),"version":version});
        if removed.contains(package) && !replaced(package) {
            blocking.push(reason("removed", "update.compat.removed", params));
        } else if !present(package) && !replaced(package) {
            blocking.push(reason("missing", "update.compat.missing", params));
        }
    }
    // the archive's gnome-shell major: the simulation's new version, else the candidate or the newest archive version
    let shell_version = installs
        .get("gnome-shell")
        .map(|v| v.to_string())
        .or_else(|| {
            let p = f.policy.get("gnome-shell")?;
            p.candidate
                .clone()
                .filter(|c| p.archive.contains(c))
                .or_else(|| p.archive.first().cloned())
        });
    let gnome_major = shell_version.as_deref().and_then(major);
    let mut notes = Vec::new();
    if let (Some(expected), Some(found)) = (&f.expected_major, &gnome_major)
        && expected != found
    {
        notes.push(format!(
            "releases.json says GNOME {expected}, the archive has {found}"
        ));
    }
    let mut degraded: Vec<Value> = Vec::new();
    for (code, packages) in DEGRADED {
        let installed: Vec<&str> = packages
            .iter()
            .copied()
            .filter(|p| f.policy.get(*p).is_some_and(|p| p.installed.is_some()))
            .collect();
        let gone = |p: &str| (removed.contains(p) || !present(p)) && !replaced(p);
        if !installed.is_empty() && installed.iter().all(|p| gone(p)) {
            let r = reason(
                "degraded",
                "update.compat.degraded",
                json!({"part":format!("update.part.{code}"),"feature":format!("update.feature.{code}"),"version":version}),
            );
            if !degraded.contains(&r) {
                degraded.push(r);
            }
        }
    }
    if f.third_party.0 > 0 {
        degraded.push(reason(
            "thirdParty",
            "update.compat.thirdParty",
            json!({"n":f.third_party.0,"name":f.third_party.1}),
        ));
    }
    let holds = !gnome_major
        .as_deref()
        .is_some_and(|m| shell_matches(&f.shell, m));
    let (state, reasons) = if !blocking.is_empty() {
        ("blocked", blocking)
    } else if holds {
        ("hold", Vec::new())
    } else if !degraded.is_empty() {
        ("warn", degraded)
    } else {
        ("ok", Vec::new())
    };
    json!({"state":state,"reasons":reasons,"gnomeMajor":gnome_major,"notes":notes})
}
/// The card's state from the archive verdict and Ubuntu's own `UpgradeBroken` (design §6.2's order).
pub(crate) fn decide(archive: &str, broken: bool) -> &str {
    match archive {
        "blocked" | "hold" | "checking" => archive,
        _ if broken => "ubuntuHold",
        _ => archive,
    }
}

// ------------------------------------------------------------------------------------------------ the full check

/// The raw outputs of one full check (the D-G27 test records them as fixtures).
#[derive(Debug, Default)]
pub(crate) struct Raw {
    pub dpkg: String,
    pub system_dump: String,
    pub tree_dump: String,
    pub update: String,
    pub policy: String,
    pub simulate: String,
    pub conf: String,
    pub sources: String,
}
/// Our win11-* binaries' NEEDED sonames (skips applied), plus libarchive.
pub(crate) fn sonames(paths: &Paths) -> Result<BTreeSet<String>, String> {
    let dir = paths.bin_dir.as_ref().ok_or("no program directory")?;
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("win11-"))
        })
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    let mut names = BTreeSet::new();
    let mut parsed = 0;
    for file in &files {
        if let Ok(needed) = elf::needed(file) {
            parsed += 1;
            names.extend(needed.into_iter().filter(|n| !elf::skipped(n)));
        }
    }
    if parsed == 0 {
        return Err(format!("no win11-* program in {}", dir.display()));
    }
    names.insert(ARCHIVE_SONAME.to_owned());
    Ok(names)
}
fn os(s: impl Into<OsString>) -> OsString {
    s.into()
}
/// The record of a check that could not finish (`unknown`), with an optional reason.
fn failed(base: &Value, reason: Option<Value>, note: &str) -> Value {
    let mut record = base.clone();
    record["state"] = json!("unknown");
    record["reasons"] = json!(reason.into_iter().collect::<Vec<_>>());
    record["notes"] = json!([note]);
    record
}
/// The full check of design §6.2 for `offer` (module doc). Returns the cache record and the raw outputs.
pub(crate) fn run_full(
    shared: &Shared,
    offer: &Offer,
    entry: &Value,
    ext: &Extension,
    now: u64,
) -> (Value, Raw) {
    let setup = &shared.setup;
    let paths = &setup.paths;
    let mut raw = Raw::default();
    let base = json!({"version":1,"codename":offer.codename,"state":"unknown","reasons":[],"gnomeMajor":null,
                      "checkedAt":now,"archiveDate":null,"inputs":inputs(entry, ext, offer),"notes":[],"blocking":[]});
    let apt = reason("apt", "update.compat.apt", json!({"version":offer.version}));
    if shared.ctx.dry_run || crate::backend::dry_run() {
        return (failed(&base, None, "dry run"), raw);
    }
    // 1. blocking packages: the owners of our sonames
    let sonames = match sonames(paths) {
        Ok(s) => s,
        Err(e) => return (failed(&base, None, &e), raw),
    };
    let args: Vec<OsString> = std::iter::once(os("-S"))
        .chain(
            sonames
                .iter()
                .map(|s| paths.soname_dir.join(s).into_os_string()),
        )
        .collect();
    let Some(found) = run(setup, Tool::DpkgQuery, &args, &[], QUERY_TIMEOUT)
        .filter(|o| matches!(o.rc, Some(0 | 1)))
    else {
        return (failed(&base, None, "dpkg-query failed"), raw);
    };
    raw.dpkg = found.stdout;
    let owners = parse_dpkg_search(&raw.dpkg);
    let mut blocking: BTreeSet<String> = sonames
        .iter()
        .filter_map(|s| owners.get(s))
        .flatten()
        .cloned()
        .collect();
    if !owners.contains_key(ARCHIVE_SONAME) {
        blocking.insert(ARCHIVE_PACKAGE.to_owned());
    }
    // 2. the tree: sources from ubuntu.sources, the system's proxy, our own APT_CONFIG
    let current = super::release::parse_os_release(
        &fs::read_to_string(&paths.os_release).unwrap_or_default(),
    )
    .codename;
    let ubuntu = fs::read_to_string(paths.apt_sources_d.join("ubuntu.sources")).unwrap_or_default();
    let stanzas = current
        .as_deref()
        .map(|c| target_stanzas(&ubuntu, c, &offer.codename, setup.test))
        .unwrap_or_default();
    if stanzas.is_empty() {
        return (
            failed(&base, Some(apt), "no Ubuntu stanza in ubuntu.sources"),
            raw,
        );
    }
    raw.system_dump = run(setup, Tool::AptConfig, &[os("dump")], &[], QUERY_TIMEOUT)
        .filter(|o| o.rc == Some(0))
        .map(|o| o.stdout)
        .unwrap_or_default();
    let proxy = proxy(&parse_config_dump(&raw.system_dump));
    let dir = tree_dir(shared, &offer.codename);
    let built = apt_conf(&dir, paths, proxy.as_deref())
        .and_then(|conf| Ok((conf, sources(&stanzas, &paths.archive_keyring)?)))
        .and_then(|(conf, list)| {
            raw.conf = conf.clone();
            raw.sources = list.clone();
            build_tree(&dir, &conf, &list)
        });
    let tree = match built {
        Ok(tree) => tree,
        Err(e) => return (failed(&base, None, &format!("tree: {e}")), raw),
    };
    let env = [("APT_CONFIG", dir.join("etc/apt/apt.conf").into_os_string())];
    // 3. the tree's configuration shows no hook of the system (C17/D-G27)
    raw.tree_dump = run(setup, Tool::AptConfig, &[os("dump")], &env, QUERY_TIMEOUT)
        .filter(|o| o.rc == Some(0))
        .map(|o| o.stdout)
        .unwrap_or_default();
    let problems = config_problems(&parse_config_dump(&raw.tree_dump), &dir);
    if !problems.is_empty() {
        return (
            failed(
                &base,
                Some(apt),
                &format!("apt-config: {}", problems.join("; ")),
            ),
            raw,
        );
    }
    // 4. the index (about 15 MB), the policy, the simulation
    let update = run(
        setup,
        Tool::AptGet,
        &[os("-q"), os("update")],
        &env,
        UPDATE_TIMEOUT,
    );
    raw.update = update
        .as_ref()
        .map(|o| format!("{}{}", o.stdout, o.stderr))
        .unwrap_or_default();
    if !update.is_some_and(|o| o.rc == Some(0) && !o.stdout.lines().any(|l| l.starts_with("Err:")))
    {
        return (failed(&base, Some(apt), "apt-get update failed"), raw);
    }
    let mut names: Vec<String> = std::iter::once("gnome-shell".to_owned())
        .chain(blocking.iter().cloned())
        .collect();
    names.extend(
        DEGRADED
            .iter()
            .flat_map(|(_, p)| p.iter().map(|p| p.to_string())),
    );
    names.retain(|n| package_name(n));
    names.dedup();
    let args: Vec<OsString> = std::iter::once(os("policy"))
        .chain(names.iter().map(os))
        .collect();
    let Some(policy) =
        run(setup, Tool::AptCache, &args, &env, QUERY_TIMEOUT).filter(|o| o.rc == Some(0))
    else {
        return (failed(&base, Some(apt), "apt-cache policy failed"), raw);
    };
    raw.policy = policy.stdout;
    let simulate = run(
        setup,
        Tool::AptGet,
        &[os("-s"), os("-q"), os("dist-upgrade")],
        &env,
        QUERY_TIMEOUT,
    );
    raw.simulate = simulate
        .as_ref()
        .map(|o| format!("{}{}", o.stdout, o.stderr))
        .unwrap_or_default();
    let sim = match simulate
        .filter(|o| o.rc == Some(0))
        .map(|o| parse_simulation(&o.stdout))
    {
        Some(Ok(sim)) => sim,
        Some(Err(e)) => return (failed(&base, Some(apt), &e), raw),
        None => {
            return (
                failed(&base, Some(apt), "apt-get -s dist-upgrade failed"),
                raw,
            );
        }
    };
    let facts = Facts {
        blocking,
        policy: parse_policy(&raw.policy),
        sim,
        third_party: third_party(&paths.apt_sources_d),
        shell: ext.shell.clone(),
        expected_major: entry["gnomeShell"].as_str().map(str::to_owned),
        version: offer.version.clone(),
    };
    let verdict = evaluate(&facts);
    let mut record = base;
    for key in ["state", "reasons", "gnomeMajor", "notes"] {
        record[key] = verdict[key].clone();
    }
    record["archiveDate"] = json!(archive_date(
        &dir.join("var/lib/apt/lists"),
        &offer.codename
    ));
    record["blocking"] = json!(facts.blocking);
    drop(tree);
    (record, raw)
}

// ------------------------------------------------------------------------------------------------ cache

/// A codename that can name a file.
fn codename_ok(codename: &str) -> bool {
    (1..=32).contains(&codename.len())
        && codename
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase())
        && codename
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
fn cache_name(codename: &str) -> String {
    format!("compat-{codename}.json")
}
/// The cached record when it still counts: same codename and inputs, younger than 7 days (a failed one: 1 hour).
pub(crate) fn valid(record: &Value, codename: &str, inputs: &Value, now: u64) -> bool {
    let age = record["checkedAt"]
        .as_u64()
        .map(|at| now.saturating_sub(at));
    let limit = if record["state"] == "unknown" {
        RETRY_MS
    } else {
        VALID_MS
    };
    record["version"] == 1
        && record["codename"] == codename
        && record["inputs"] == *inputs
        && age.is_some_and(|a| a < limit)
}
fn read_cache(
    shared: &Shared,
    offer: &Offer,
    entry: &Value,
    ext: &Extension,
    now: u64,
) -> Option<Value> {
    if !codename_ok(&offer.codename) {
        return None;
    }
    shared
        .store
        .read(&cache_name(&offer.codename))
        .filter(|r| valid(r, &offer.codename, &inputs(entry, ext, offer), now))
}
/// The offered release (files only).
fn offered(shared: &Shared) -> Option<Offer> {
    let paths = &shared.setup.paths;
    if cfg!(test) && !paths.fixture {
        return None; // unit tests never read this machine's release files
    }
    super::release::detect(paths, &super::release::today()).offer
}

/// The verdict known without running anything (GW5's `update.release.ubuntuHold` checks it): the cheap hold, or
/// the valid `compat-<codename>.json` (its archive state; `ubuntuHold` is not part of it). `{state, source,
/// gnomeMajor, checkedAt}` or None.
pub(super) fn cached(shared: &Shared, codename: &str) -> Option<Value> {
    let paths = &shared.setup.paths;
    let releases = releases(paths);
    let entry = entry(&releases, codename);
    let ext = extension(paths, &releases);
    if cheap(&entry, &ext) == Cheap::Hold {
        return Some(json!({"state":"hold","source":"cheap","gnomeMajor":null,"checkedAt":null}));
    }
    let offer = offered(shared).filter(|o| o.codename == codename)?;
    let record = read_cache(shared, &offer, &entry, &ext, store::now_ms())?;
    Some(
        json!({"state":record["state"],"source":"archive","gnomeMajor":record["gnomeMajor"],
                "checkedAt":record["checkedAt"]}),
    )
}

/// A stored reason in `lang`: `{code, key, params, message}` (part and feature texts are keys, rendered here).
pub(crate) fn render(reason: &Value, lang: Lang) -> Value {
    let key = reason["key"].as_str().unwrap_or("");
    let mut params: Map<String, Value> = reason["params"].as_object().cloned().unwrap_or_default();
    for field in ["part", "feature"] {
        if let Some(text) = params.get(field).and_then(Value::as_str).map(str::to_owned) {
            let shown = if text.starts_with("update.part.") || text.starts_with("update.feature.") {
                crate::i18n::tr(lang, &text)
            } else {
                text
            };
            params.insert(field.to_owned(), json!(shown));
        }
    }
    let filled = Value::Object(params);
    let message = if reason["code"] == "thirdParty" {
        crate::i18n::trn(lang, key, filled["n"].as_u64().unwrap_or(0), &filled)
    } else {
        crate::i18n::trf(lang, key, &filled)
    };
    json!({"code":reason["code"],"key":key,"params":reason["params"],"message":message})
}
/// Does ubuntu-release-upgrader have a Dutch catalogue (else the start dialog says the first lines are English)?
pub(crate) fn upgrader_nl(paths: &Paths) -> bool {
    paths
        .locale_dirs
        .iter()
        .any(|dir| dir.join(UPGRADER_MO).is_file())
}

/// The card's fields of the snapshot's `release` object (release::status, for every host; files only):
/// `state` and `verdict` (none|checking|ok|warn|hold|ubuntuHold|blocked|unknown), `reasons` (rendered),
/// `gnomeMajor` (the archive's, never releases.json's), `allowAnyway`, `compat {running, needed, checkedAt}`,
/// `upgraderNl`. GW7's later states override `state`; `verdict` stays GW6's.
pub(super) fn fill(shared: &Shared, release: &mut Value, offer: Option<&Offer>) {
    let paths = &shared.setup.paths;
    let lang = shared.ctx.lang();
    release["verdict"] = json!("none");
    release["allowAnyway"] = json!(false);
    release["compat"] = json!({"running":false,"needed":false,"checkedAt":null});
    release["upgraderNl"] = json!(upgrader_nl(paths));
    let Some(offer) = offer else { return };
    let releases = releases(paths);
    let entry = entry(&releases, &offer.codename);
    let ext = extension(paths, &releases);
    release["allowAnyway"] = json!(entry["allowAnyway"] == true);
    let running = shared.store.locked(LOCK);
    let broken = offer.broken.is_some();
    let now = store::now_ms();
    let (state, reasons, major, checked, needed) = if cheap(&entry, &ext) == Cheap::Hold {
        (
            "hold".to_owned(),
            Vec::new(),
            Value::Null,
            Value::Null,
            false,
        )
    } else if let Some(record) = read_cache(shared, offer, &entry, &ext, now)
        .filter(|r| !(running && r["state"] == "unknown"))
    {
        let state = decide(record["state"].as_str().unwrap_or("unknown"), broken).to_owned();
        let reasons = if state == "ubuntuHold" {
            Vec::new()
        } else {
            record["reasons"].as_array().cloned().unwrap_or_default()
        };
        (
            state,
            reasons,
            record["gnomeMajor"].clone(),
            record["checkedAt"].clone(),
            false,
        )
    } else if running {
        (
            "checking".to_owned(),
            Vec::new(),
            Value::Null,
            Value::Null,
            false,
        )
    } else {
        // nothing known: the page asks for the check, unless it cannot run now (offline, metered, dry run)
        let snapshot = shared.store.read(store::SNAPSHOT).unwrap_or(Value::Null);
        let network = net::Net::from_json(&snapshot["network"]);
        let refusal = if shared.ctx.dry_run || crate::backend::dry_run() {
            Some(None)
        } else if !network.online {
            Some(Some(reason("offline", "update.compat.offline", json!({}))))
        } else if network.metered && shared.ctx.settings()["update"]["metered"] != true {
            Some(Some(reason("metered", "update.compat.metered", json!({}))))
        } else {
            None
        };
        match refusal {
            Some(why) => (
                decide("unknown", broken).to_owned(),
                why.into_iter().collect(),
                Value::Null,
                Value::Null,
                false,
            ),
            None => (
                "checking".to_owned(),
                Vec::new(),
                Value::Null,
                Value::Null,
                true,
            ),
        }
    };
    release["state"] = json!(state);
    release["verdict"] = json!(state);
    release["reasons"] = json!(reasons.iter().map(|r| render(r, lang)).collect::<Vec<_>>());
    release["gnomeMajor"] = major;
    release["compat"] = json!({"running":running,"needed":needed,"checkedAt":checked});
}

// ------------------------------------------------------------------------------------------------ the method

/// `update.changed` after a compat change (release.rs's rule: the next `rev`, not written; design §4.4).
fn announce(shared: &Shared) {
    let snapshot = shared.store.read(store::SNAPSHOT);
    let job = shared.store.read(store::JOB);
    let view = super::compose(shared, snapshot.as_ref(), job.as_ref(), None, false);
    let rev = store::next_rev(&shared.store, shared.rev.load(Ordering::SeqCst));
    shared.ctx.emit(
        "update.changed",
        json!({"rev":rev,"phase":view["phase"],"totals":view["totals"],"restart":view["restart"],"job":view["job"],
               "error":view["error"],"release":{"state":view["release"]["state"],"version":view["release"]["version"]}}),
    );
}
fn skipped(why: &str) -> ApiResult {
    Ok(json!({"started":false,"skipped":why}))
}
/// `update.release.compat {refresh?:bool}` → `{started:true}` or `{started:false, skipped}` (module doc).
pub(super) fn compat(shared: &Shared, args: &Value, _lang: Lang) -> ApiResult {
    if shared.ctx.dry_run || crate::backend::dry_run() {
        return Ok(json!({"state":"unknown","dryRun":true}));
    }
    let Some(offer) = offered(shared).filter(|o| codename_ok(&o.codename)) else {
        return skipped("notOffered");
    };
    let paths = &shared.setup.paths;
    let releases = releases(paths);
    let entry = entry(&releases, &offer.codename);
    let ext = extension(paths, &releases);
    if cheap(&entry, &ext) == Cheap::Hold {
        return skipped("hold");
    }
    let now = store::now_ms();
    if let Some(record) = read_cache(shared, &offer, &entry, &ext, now)
        && !(args["refresh"] == true && record["state"] == "unknown")
    {
        return skipped("recent");
    }
    let snapshot = shared.store.read(store::SNAPSHOT).unwrap_or(Value::Null);
    let network = net::Net::from_json(&snapshot["network"]);
    if !network.online {
        return skipped("offline");
    }
    if network.metered && shared.ctx.settings()["update"]["metered"] != true {
        return skipped("metered");
    }
    let Ok(Some(lock)) = shared.store.try_lock(LOCK) else {
        return skipped("running");
    };
    // the thread's own view of the service (the pk thread keeps the original), as release.rs's check does
    let copy = Shared {
        ctx: shared.ctx.clone(),
        setup: shared.setup.clone(),
        store: shared.store.clone(),
        stats: Stats::default(),
        rev: AtomicU64::new(shared.rev.load(Ordering::SeqCst)),
    };
    shared.stats.threads.fetch_add(1, Ordering::SeqCst);
    let spawned = std::thread::Builder::new()
        .name("update-compat".into())
        .spawn(move || {
            announce(&copy);
            let (record, _) = run_full(&copy, &offer, &entry, &ext, store::now_ms());
            let _ = copy.store.write(&cache_name(&offer.codename), &record);
            if matches!(record["state"].as_str(), Some("ok" | "warn")) {
                // Ubuntu's own prompt goes back once we are ready for this release (§4.12)
                let _ = super::release::restore_hold(&copy);
            }
            drop(lock);
            announce(&copy);
        });
    if spawned.is_err() {
        return skipped("running");
    }
    Ok(json!({"started":true}))
}

// ------------------------------------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::update::{
        admin::tests::{fixture as release_fixture, shared},
        tests::{Fixture, script, write},
    };
    use std::os::unix::fs::PermissionsExt;

    const TESTED: &str = r#"{"version":1,"extension":"win11-gnome@win11shell.local","releases":{
        "stonking":{"version":"26.10","gnomeShell":"50","status":"tested","allowAnyway":false}}}"#;

    /// A file of tests/update/fixtures/compat (read at run time: the source tarball has no tests/).
    fn fixture(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/update/fixtures/compat")
            .join(name);
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }
    fn fixture_path(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/update/fixtures/compat")
            .join(name)
    }
    fn offer(codename: &str, broken: bool) -> Offer {
        Offer {
            version: "26.10".into(),
            codename: codename.into(),
            name: Some("Stonking Stingray".into()),
            notes: None,
            broken: broken.then(|| "https://discourse.ubuntu.com/t/upgrade-broken".into()),
            eol: Some("2027-07-15".into()),
            support_months: Some(9),
            source: "cache",
        }
    }
    fn ext(shell: &[&str]) -> Extension {
        Extension {
            version: Some("8".into()),
            shell: shell.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn the_embedded_releases_json_is_the_designs() {
        let r: Value = serde_json::from_str(RELEASES).unwrap();
        assert_eq!(r["version"], 1);
        assert_eq!(r["extension"], "win11-gnome@win11shell.local");
        assert_eq!(
            r["releases"]["resolute"],
            json!({"version":"26.04","gnomeShell":"50","status":"tested"})
        );
        assert_eq!(
            r["releases"]["stonking"],
            json!({"version":"26.10","gnomeShell":"51","status":"untested","allowAnyway":false})
        );
    }

    #[test]
    fn majors_and_gnomes_prefix_rule() {
        assert_eq!(major("51.0-0ubuntu1").as_deref(), Some("51"));
        assert_eq!(major("1:2.3").as_deref(), Some("2"));
        assert_eq!(major("50.1-0ubuntu1.5").as_deref(), Some("50"));
        assert_eq!(major("rc1"), None);
        let shell: Vec<String> = ["50", "51.1"].iter().map(|s| s.to_string()).collect();
        assert!(shell_matches(&shell, "50") && shell_matches(&shell, "51"));
        assert!(
            !shell_matches(&shell, "5")
                && !shell_matches(&shell, "510")
                && !shell_matches(&shell, "")
        );
    }

    /// C13/C16: only a tested codename whose major the installed shell-version names gets the full check.
    #[test]
    fn the_cheap_verdict_matrix() {
        let tested = |major: &str| json!({"version":"26.10","gnomeShell":major,"status":"tested"});
        let cases = [
            (tested("50"), ext(&["50"]), Cheap::Full),
            (tested("50"), ext(&["49", "50"]), Cheap::Full),
            (tested("51"), ext(&["50"]), Cheap::Hold),
            (
                json!({"version":"26.10","gnomeShell":"50","status":"untested"}),
                ext(&["50"]),
                Cheap::Hold,
            ),
            (
                json!({"version":"26.10","gnomeShell":"50"}),
                ext(&["50"]),
                Cheap::Hold,
            ),
            (Value::Null, ext(&["50"]), Cheap::Hold),
            (tested("50"), Extension::default(), Cheap::Hold),
        ];
        for (entry, ext, want) in cases {
            assert_eq!(cheap(&entry, &ext), want, "{entry} {ext:?}");
        }
        // today: the embedded file holds stonking (untested, GNOME 51)
        let embedded: Value = serde_json::from_str(RELEASES).unwrap();
        assert_eq!(
            cheap(&entry(&embedded, "stonking"), &ext(&["50"])),
            Cheap::Hold
        );
        assert_eq!(
            cheap(&entry(&embedded, "resolute"), &ext(&["50"])),
            Cheap::Full
        );
    }

    #[test]
    fn components_come_from_the_ubuntu_stanzas_of_ubuntu_sources() {
        let text = fixture("root/etc/apt/sources.list.d/ubuntu.sources");
        let got = target_stanzas(&text, "resolute", "stonking", true);
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(
            got[0].suites,
            ["stonking", "stonking-updates"],
            "backports left out"
        );
        assert_eq!(got[1].suites, ["stonking-security"]);
        assert_eq!(
            got[0].components,
            ["main", "restricted", "universe", "multiverse"]
        );
        assert_eq!(
            got[1].components,
            ["main", "restricted", "universe"],
            "each stanza's own components"
        );
        // Robin's machine shape: http mirrors, only the Ubuntu stanzas, never a file: URI outside test mode
        let real = "Types: deb\nURIs: http://nl.archive.ubuntu.com/ubuntu/\nSuites: resolute resolute-updates resolute-backports\n\
                    Components: main restricted universe multiverse\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n\n\
                    Types: deb\nURIs: http://security.ubuntu.com/ubuntu/\nSuites: resolute-security\n\
                    Components: main restricted universe multiverse\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n\n\
                    # Types: deb-src\nTypes: deb-src\nURIs: http://nl.archive.ubuntu.com/ubuntu/\nSuites: resolute\nComponents: main\n\n\
                    Enabled: no\nTypes: deb\nURIs: http://nl.archive.ubuntu.com/ubuntu/\nSuites: resolute-proposed\nComponents: main\n";
        let got = target_stanzas(real, "resolute", "stonking", false);
        assert_eq!(
            got,
            [
                Stanza {
                    uris: vec!["http://nl.archive.ubuntu.com/ubuntu/".into()],
                    suites: vec!["stonking".into(), "stonking-updates".into()],
                    components: ["main", "restricted", "universe", "multiverse"]
                        .map(String::from)
                        .to_vec(),
                },
                Stanza {
                    uris: vec!["http://security.ubuntu.com/ubuntu/".into()],
                    suites: vec!["stonking-security".into()],
                    components: ["main", "restricted", "universe", "multiverse"]
                        .map(String::from)
                        .to_vec(),
                },
            ]
        );
        assert!(
            target_stanzas(&text, "resolute", "stonking", false).is_empty(),
            "file: only in test mode"
        );
        let evil = "Types: deb\nURIs: http://x/\"\nSuites: resolute\nComponents: main\n";
        assert!(target_stanzas(evil, "resolute", "stonking", false).is_empty());
        let only = "Types: deb\nURIs: http://a/\nSuites: resolute-backports\nComponents: main\n";
        assert!(target_stanzas(only, "resolute", "stonking", false).is_empty());
    }

    #[test]
    fn third_party_sources_are_counted_without_ubuntu_sources() {
        let dir = fixture_path("root/etc/apt/sources.list.d");
        assert_eq!(third_party(&dir), (2, Some("dl.google.com".into())));
        let f = Fixture::new("compat-third");
        let d = f.root.join("sl");
        write(
            &d.join("ubuntu.sources"),
            &fixture("root/etc/apt/sources.list.d/ubuntu.sources"),
        );
        assert_eq!(third_party(&d), (0, None));
        write(
            &d.join("a.list"),
            "# deb http://x/ y main\ndeb-src http://x/ y main\n",
        );
        write(&d.join("b.list.save"), "deb http://x/ y main\n");
        write(
            &d.join("c.sources"),
            "Enabled: no\nTypes: deb\nURIs: https://repo.example/\nSuites: x\nComponents: main\n",
        );
        write(
            &d.join("d.list"),
            "deb http://nl.archive.ubuntu.com/ubuntu stonking main\n",
        );
        assert_eq!(
            third_party(&d),
            (0, None),
            "comments, deb-src, other extensions, disabled, Ubuntu hosts"
        );
        write(
            &d.join("e.list"),
            "deb [arch=amd64 signed-by=/k.gpg] https://ppa.launchpadcontent.net/x/y/ubuntu stonking main\n",
        );
        assert_eq!(
            third_party(&d),
            (1, Some("ppa.launchpadcontent.net".into()))
        );
    }

    #[test]
    fn the_tree_configuration() {
        let f = Fixture::new("compat-conf");
        let setup = crate::services::update::paths::resolve(&f.env_no_bus()).unwrap();
        let tree = f.base.join("tree");
        let conf = apt_conf(&tree, &setup.paths, Some("http://proxy.example:3128/")).unwrap();
        let t = tree.display().to_string();
        for line in [
            format!("Dir \"{t}/\";"),
            format!("Dir::Etc::Parts \"{t}/etc/apt/apt.conf.d/\";"),
            format!("Dir::Etc::SourceParts \"{t}/etc/apt/sources.list.d/\";"),
            format!(
                "Dir::State::status \"{}\";",
                f.root.join("var/lib/dpkg/status").display()
            ),
            "Debug::NoLocking \"1\";".into(),
            "APT::Architectures { \"amd64\"; };".into(),
            "Acquire::Languages \"none\";".into(),
            "Acquire::http::Proxy \"http://proxy.example:3128/\";".into(),
        ] {
            assert!(conf.lines().any(|l| l == line), "{line}\n{conf}");
        }
        assert!(
            !apt_conf(&tree, &setup.paths, Some("http://x\";Dir \"/"))
                .unwrap()
                .contains("Proxy")
        );
        assert!(apt_conf(Path::new("/tmp/a\"b"), &setup.paths, None).is_err());
        // the system's proxy, as apt-config dump prints it
        let dump = parse_config_dump(
            "APT \"\";\nAcquire::http::Proxy \"http://10.0.0.1:3142/\";\nDir \"/\";\n",
        );
        assert_eq!(proxy(&dump).as_deref(), Some("http://10.0.0.1:3142/"));
        assert_eq!(proxy(&parse_config_dump("Dir \"/\";\n")), None);
        let list = sources(
            &target_stanzas(
                &fixture("root/etc/apt/sources.list.d/ubuntu.sources"),
                "resolute",
                "stonking",
                true,
            ),
            Path::new("/k/r.gpg"),
        )
        .unwrap();
        assert!(
            list.contains("Suites: stonking stonking-updates\n")
                && list.contains("Signed-By: /k/r.gpg\n"),
            "{list}"
        );
        assert!(!list.contains("resolute"));
    }

    /// The recorded `apt-config dump` of the D-G27 run: the system's (fixture /etc/apt with Ubuntu's hooks) shows
    /// them, the tree's shows none and keeps every directory inside the tree.
    #[test]
    fn apt_config_dump_under_the_tree_shows_no_system_hook() {
        let tree = Path::new("/run/tree");
        let system = parse_config_dump(&fixture("recorded/apt-config-system.txt"));
        assert!(!config_problems(&system, tree).is_empty());
        assert!(
            system.iter().any(|(k, _)| k.contains("Post-Invoke")),
            "the fixture system config has hooks"
        );
        let recorded = fixture("recorded/apt-config-tree.txt").replace("@TREE@", "/run/tree");
        assert_eq!(
            config_problems(&parse_config_dump(&recorded), tree),
            Vec::<String>::new()
        );
        let hooked = format!("{recorded}DPkg::Post-Invoke:: \"/opt/hook/x\";\n");
        assert_eq!(config_problems(&parse_config_dump(&hooked), tree).len(), 1);
        let outside = recorded.replace(
            "Dir::Etc::parts \"/run/tree/etc/apt/apt.conf.d/\";",
            "Dir::Etc::parts \"/etc/apt/apt.conf.d/\";",
        );
        assert_eq!(
            config_problems(&parse_config_dump(&outside), tree).len(),
            1,
            "{outside}"
        );
    }

    #[test]
    fn dpkg_query_policy_and_simulation_parsers() {
        let owners = parse_dpkg_search(
            "libgtk-3-0t64:amd64: /usr/lib/x86_64-linux-gnu/libgtk-3.so.0\ndiversion by x from: /usr/lib/y\n\
             libglib2.0-0t64:amd64, libglib2.0-0t64:i386: /usr/lib/x86_64-linux-gnu/libglib-2.0.so.0\nnot a line\n",
        );
        assert_eq!(
            owners["libgtk-3.so.0"],
            BTreeSet::from(["libgtk-3-0t64".to_owned()])
        );
        assert_eq!(
            owners["libglib-2.0.so.0"],
            BTreeSet::from(["libglib2.0-0t64".to_owned()])
        );
        assert_eq!(owners.len(), 2);
        let policy = parse_policy(
            &[
                "gnome-shell:",
                "  Installed: 50.1-0ubuntu1.5",
                "  Candidate: 51.0-0ubuntu1",
                "  Version table:",
                "     51.0-0ubuntu1 500",
                "        500 file:/a stonking/main amd64 Packages",
                " *** 50.1-0ubuntu1.5 100",
                "        100 /var/lib/dpkg/status",
                "ffmpeg:",
                "  Installed: 7:7.1.1-1ubuntu1",
                "  Candidate: 7:7.1.1-1ubuntu1",
                "  Version table:",
                " *** 7:7.1.1-1ubuntu1 100",
                "        100 /var/lib/dpkg/status",
                "flatpak:",
                "  Installed: (none)",
                "  Candidate: (none)",
                "  Version table:",
            ]
            .join("\n"),
        );
        assert_eq!(policy["gnome-shell"].archive, ["51.0-0ubuntu1"]);
        assert_eq!(
            policy["gnome-shell"].installed.as_deref(),
            Some("50.1-0ubuntu1.5")
        );
        assert!(policy["ffmpeg"].archive.is_empty() && policy["ffmpeg"].installed.is_some());
        assert_eq!(policy["flatpak"], Policy::default());
        let sim = parse_simulation(
            "Reading package lists...\nInst gnome-shell [50.1-0ubuntu1.5] (51.0-0ubuntu1 Ubuntu:26.10/stonking [amd64])\n\
             Inst libgtk-3-0t65 (3.24.60-1 Ubuntu:26.10/stonking [amd64]) []\nRemv libgtk-3-0t64 [3.24.52-1]\n\
             Conf gnome-shell (51.0-0ubuntu1 Ubuntu:26.10/stonking [amd64])\n",
        )
        .unwrap();
        assert_eq!(
            sim,
            [
                Sim::Inst {
                    name: "gnome-shell".into(),
                    new: "51.0-0ubuntu1".into()
                },
                Sim::Inst {
                    name: "libgtk-3-0t65".into(),
                    new: "3.24.60-1".into()
                },
                Sim::Remv {
                    name: "libgtk-3-0t64".into()
                },
            ]
        );
        assert!(
            parse_simulation("E: Error, pkgProblemResolver::Resolve generated breaks\n").is_err()
        );
        assert_eq!(stem("libgtk-3-0t64"), "libgtk-3-0");
        assert_eq!(stem("libgtk-3-0t65"), "libgtk-3-0");
        assert_eq!(stem("libgtk-4-1"), "libgtk-4-1");
        assert_eq!(stem("zlib1g"), "zlib1g");
        // the part texts: GTK 4 with the libraries that come with it, else the package name
        for package in ["libgtk-4-1", "libgraphene-1.0-0", "libpango-1.0-0"] {
            assert_eq!(part_of(package), "update.part.gtk4", "{package}");
        }
        assert_eq!(part_of("libgtk-3-0t64"), "update.part.gtk");
        assert_eq!(part_of("libfoo1"), "libfoo1");
        assert!(
            package_name("libglib2.0-0t64")
                && !package_name("-o")
                && !package_name("a b")
                && !package_name("A")
        );
    }

    /// Synthetic algorithm data: the outputs under tests/update/fixtures/compat/recorded/ are the real apt's answers
    /// for the throwaway GW6 archives of tests/update/mkarchive.py (made-up versions; their packages are Robin's
    /// 26.04 of then, WebKitGTK included), and this blocking set is the one of the binaries of that time. They test
    /// the verdict rules, not our programs' dependencies (test_compat.sh's layer test reads the real binaries).
    fn facts(case: &str) -> Facts {
        let blocking = [
            "libcairo2",
            "libgdk-pixbuf-2.0-0",
            "libglib2.0-0t64",
            "libgtk-3-0t64",
            "libjavascriptcoregtk-4.1-0",
            "libsoup-3.0-0",
            "libwebkit2gtk-4.1-0",
            "zlib1g",
            "libarchive13t64",
        ];
        Facts {
            blocking: blocking.iter().map(|s| s.to_string()).collect(),
            policy: parse_policy(&fixture(&format!("recorded/{case}/policy.txt"))),
            sim: parse_simulation(&fixture(&format!("recorded/{case}/simulate.txt"))).unwrap(),
            third_party: if case == "warn" {
                (2, Some("dl.google.com".into()))
            } else {
                (0, None)
            },
            shell: vec!["50".into()],
            expected_major: Some("50".into()),
            version: "26.10".into(),
        }
    }
    /// The verdict matrix on the outputs recorded from the real apt (D-G27, tests/update/test_compat.sh).
    #[test]
    fn the_verdict_matrix_on_recorded_apt_outputs() {
        let keys = |v: &Value| {
            v["reasons"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| {
                    format!(
                        "{}:{}",
                        r["key"].as_str().unwrap(),
                        r["params"]["part"].as_str().unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
        };
        let ok = evaluate(&facts("ok"));
        assert_eq!(
            (ok["state"].clone(), ok["gnomeMajor"].clone()),
            (json!("ok"), json!("50")),
            "{ok}"
        );
        let warn = evaluate(&facts("warn"));
        assert_eq!(warn["state"], "warn", "{warn}");
        assert_eq!(
            keys(&warn),
            [
                "update.compat.degraded:update.part.ffmpeg",
                "update.compat.thirdParty:"
            ]
        );
        let hold = evaluate(&facts("hold"));
        assert_eq!(
            (hold["state"].clone(), hold["gnomeMajor"].clone()),
            (json!("hold"), json!("51")),
            "{hold}"
        );
        assert_eq!(
            hold["notes"][0],
            "releases.json says GNOME 50, the archive has 51"
        );
        let blocked = evaluate(&facts("blocked"));
        assert_eq!(blocked["state"], "blocked", "{blocked}");
        assert_eq!(
            keys(&blocked),
            [
                "update.compat.removed:update.part.archive",
                "update.compat.missing:update.part.jsc",
                "update.compat.missing:update.part.webkit"
            ]
        );
        // the renamed GTK (t64 → t65) of the warn case is not a missing part
        assert!(!keys(&warn).iter().any(|k| k.contains("gtk")));
        // ubuntuHold: the archive is fine, Ubuntu paused the upgrade (applied when the snapshot is read)
        let paused = evaluate(&facts("ubuntuHold"));
        assert_eq!(
            decide(paused["state"].as_str().unwrap(), true),
            "ubuntuHold"
        );
        assert_eq!(decide("blocked", true), "blocked");
        assert_eq!(decide("hold", true), "hold");
        assert_eq!(decide("unknown", true), "ubuntuHold");
        assert_eq!(decide("warn", false), "warn");
    }

    #[test]
    fn reasons_are_rendered_per_call() {
        let r = reason(
            "missing",
            "update.compat.missing",
            json!({"part":"update.part.webkit","version":"26.10"}),
        );
        assert_eq!(
            render(&r, Lang::Nl)["message"],
            "WebKitGTK (webweergave) zit niet in Ubuntu 26.10."
        );
        assert_eq!(
            render(&r, Lang::En)["message"],
            "WebKitGTK (web view) isn’t in Ubuntu 26.10."
        );
        assert_eq!(
            render(&r, Lang::En)["params"]["part"],
            "update.part.webkit",
            "params keep the key"
        );
        let raw = reason(
            "removed",
            "update.compat.removed",
            json!({"part":"libfoo1","version":"26.10"}),
        );
        assert_eq!(
            render(&raw, Lang::En)["message"],
            "Ubuntu 26.10 removes libfoo1."
        );
        let d = reason(
            "degraded",
            "update.compat.degraded",
            json!({"part":"update.part.ffmpeg","feature":"update.feature.ffmpeg","version":"26.10"}),
        );
        assert_eq!(
            render(&d, Lang::Nl)["message"],
            "ffmpeg ontbreekt in Ubuntu 26.10; video-miniaturen werkt dan niet."
        );
        let one = reason(
            "thirdParty",
            "update.compat.thirdParty",
            json!({"n":1,"name":"dl.google.com"}),
        );
        assert_eq!(
            render(&one, Lang::En)["message"],
            "Ubuntu turns off 1 extra software source during the upgrade (dl.google.com)."
        );
        let two = reason(
            "thirdParty",
            "update.compat.thirdParty",
            json!({"n":2,"name":"dl.google.com"}),
        );
        assert_eq!(
            render(&two, Lang::Nl)["message"],
            "Ubuntu zet tijdens de upgrade 2 extra softwarebronnen uit, zoals dl.google.com."
        );
    }

    // ------------------------------------------------------------------------------------ the service around it

    /// This machine's release files (design §1.2) with stonking offered, our extension, and stub tools that replay
    /// the recorded apt outputs of `case`.
    fn offered_fixture(f: &Fixture, case: &str, releases: Option<&str>, broken: bool) {
        write(
            &f.root.join("etc/os-release"),
            &release_fixture("../compat/root/etc/os-release"),
        );
        write(
            &f.root.join("usr/share/distro-info/ubuntu.csv"),
            &release_fixture("distro-info/ubuntu.csv"),
        );
        write(
            &f.root.join("etc/update-manager/release-upgrades"),
            &release_fixture("etc/update-manager/release-upgrades"),
        );
        let meta = if broken {
            "meta-release/stonking-broken"
        } else {
            "meta-release/stonking-supported"
        };
        write(
            &f.root.join("home/.cache/update-manager-core/meta-release"),
            &release_fixture(meta),
        );
        write(
            &f.root.join("home/.local/share/gnome-shell/extensions/win11-gnome@win11shell.local/metadata.json"),
            &fixture("root/home/.local/share/gnome-shell/extensions/win11-gnome@win11shell.local/metadata.json"),
        );
        for name in ["ubuntu.sources", "google-chrome.sources", "vscode.list"] {
            let text = fixture(&format!("root/etc/apt/sources.list.d/{name}"));
            if case == "warn" || name == "ubuntu.sources" {
                write(&f.root.join("etc/apt/sources.list.d").join(name), &text);
            }
        }
        if let Some(text) = releases {
            write(&f.root.join("releases.json"), text);
        }
        // our programs: a tiny ELF with win11-app's recorded NEEDED (no build needed)
        let recorded = fixture("readelf-needed.txt");
        let needed: Vec<&str> = recorded
            .lines()
            .find_map(|l| l.strip_prefix("win11-app: "))
            .unwrap()
            .split_whitespace()
            .collect();
        write(&f.root.join("bin/README"), "not a program\n");
        fs::write(
            f.root.join("bin/win11-app"),
            super::super::elf::tests::tiny(&needed, &[]),
        )
        .unwrap();
        let rec = fixture_path(&format!("recorded/{case}"));
        let common = fixture_path("recorded");
        let cat = |file: &Path| {
            format!(
                "while IFS= read -r l; do printf '%s\\n' \"$l\"; done < '{}'\n",
                file.display()
            )
        };
        script(
            &f.tools.join("dpkg-query"),
            &format!(
                "echo \"dpkg-query $*\" >> '{}'\n{}",
                f.log.display(),
                cat(&fixture_path("dpkg-query-S.txt"))
            ),
        );
        script(
            &f.tools.join("apt-get"),
            &format!(
                "echo \"apt-get $* APT_CONFIG=$APT_CONFIG\" >> '{log}'\ncase \"$*\" in\n  *update*) {u} ;;\n  *dist-upgrade*) {s} ;;\nesac\n",
                log = f.log.display(),
                u = cat(&rec.join("update.txt")).trim_end(),
                s = cat(&rec.join("simulate.txt")).trim_end()
            ),
        );
        script(
            &f.tools.join("apt-cache"),
            &format!(
                "echo \"apt-cache $* APT_CONFIG=$APT_CONFIG\" >> '{}'\n{}",
                f.log.display(),
                cat(&rec.join("policy.txt"))
            ),
        );
        let tree_dump = common.join("apt-config-tree.txt");
        fs::write(
            f.tools.join("apt-config"),
            format!(
                "#!/bin/bash\necho \"apt-config $* APT_CONFIG=$APT_CONFIG\" >> '{log}'\nif [ -n \"$APT_CONFIG\" ]; then t=${{APT_CONFIG%/etc/apt/apt.conf}}; \
                 while IFS= read -r l; do printf '%s\\n' \"${{l//@TREE@/$t}}\"; done < '{tree}'; else {sys} fi\n",
                log = f.log.display(),
                tree = tree_dump.display(),
                sys = cat(&common.join("apt-config-system.txt"))
            ),
        )
        .unwrap();
        fs::set_permissions(
            f.tools.join("apt-config"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }

    #[test]
    fn a_non_tested_codename_never_builds_the_tree() {
        let f = Fixture::new("compat-cheap");
        offered_fixture(&f, "ok", None, false); // the embedded releases.json: stonking untested, GNOME 51
        let shared = shared(&f, "nl");
        assert_eq!(
            compat(&shared, &json!({}), Lang::Nl).unwrap(),
            json!({"started":false,"skipped":"hold"})
        );
        assert_eq!(
            compat(&shared, &json!({"refresh":true}), Lang::Nl).unwrap()["skipped"],
            "hold"
        );
        let mut release = json!({});
        fill(&shared, &mut release, offered(&shared).as_ref());
        assert_eq!(
            (release["state"].clone(), release["gnomeMajor"].clone()),
            (json!("hold"), Value::Null)
        );
        assert_eq!(
            release["compat"],
            json!({"running":false,"needed":false,"checkedAt":null})
        );
        assert_eq!(
            cached(&shared, "stonking").unwrap()["state"],
            "hold",
            "GW5's ubuntuHold may hold it"
        );
        assert_eq!(shared.setup.tools.runs(), 0, "no program ran");
        assert_eq!(f.stub_log(), "");
        assert!(!f.base.join("cache/update").join(TREE).exists(), "no tree");
        // a matching major that is not tested holds too
        write(
            &f.root.join("releases.json"),
            &TESTED.replace("\"tested\"", "\"untested\""),
        );
        assert_eq!(
            compat(&shared, &json!({}), Lang::Nl).unwrap()["skipped"],
            "hold"
        );
        assert_eq!(shared.setup.tools.runs(), 0);
    }

    /// Wait for the check's thread: its lock goes once the record is written.
    fn finished(shared: &Shared) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while shared.store.locked(LOCK) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!shared.store.locked(LOCK), "the check did not end");
    }

    #[test]
    fn a_tested_codename_runs_the_full_check_once_and_caches_it() {
        for (case, want) in [
            ("ok", "ok"),
            ("warn", "warn"),
            ("hold", "hold"),
            ("blocked", "blocked"),
            ("ubuntuHold", "ubuntuHold"),
        ] {
            let f = Fixture::new("compat-full");
            offered_fixture(&f, case, Some(TESTED), case == "ubuntuHold");
            let shared = shared(&f, "en");
            let mut before = json!({});
            fill(&shared, &mut before, offered(&shared).as_ref());
            assert_eq!(before["state"], "checking", "{case}");
            assert_eq!(
                before["compat"]["needed"], true,
                "{case}: the page asks for the check"
            );
            assert_eq!(
                compat(&shared, &json!({}), Lang::En).unwrap(),
                json!({"started":true}),
                "{case}"
            );
            finished(&shared);
            let record = shared.store.read("compat-stonking.json").unwrap();
            let mut release = json!({});
            fill(&shared, &mut release, offered(&shared).as_ref());
            assert_eq!(release["state"], want, "{case}: {record}");
            assert_eq!(release["compat"]["needed"], false, "{case}");
            assert!(
                record["checkedAt"].is_u64() && record["inputs"]["entry"]["status"] == "tested",
                "{case}"
            );
            let log = f.stub_log();
            assert_eq!(log.matches("apt-get -q update").count(), 1, "{case}: {log}");
            assert!(
                log.contains("apt-cache policy gnome-shell")
                    && log.contains("apt-get -s -q dist-upgrade"),
                "{case}: {log}"
            );
            assert!(log.contains("dpkg-query -S"), "{case}");
            // every apt call ran with the tree's own configuration
            for line in log
                .lines()
                .filter(|l| l.starts_with("apt-get") || l.starts_with("apt-cache"))
            {
                assert!(
                    line.contains("/update/release-check/stonking/etc/apt/apt.conf"),
                    "{case}: {line}"
                );
            }
            assert!(
                !f.base
                    .join("cache/update")
                    .join(TREE)
                    .join("stonking")
                    .exists(),
                "{case}: the tree is deleted"
            );
            // the 7-day rule: a second call (also with refresh) runs nothing
            let runs = shared.setup.tools.runs();
            assert_eq!(
                compat(&shared, &json!({}), Lang::En).unwrap()["skipped"],
                "recent",
                "{case}"
            );
            assert_eq!(
                compat(&shared, &json!({"refresh":true}), Lang::En).unwrap()["skipped"],
                "recent",
                "{case}"
            );
            assert_eq!(shared.setup.tools.runs(), runs, "{case}");
            if case == "blocked" {
                assert_eq!(
                    release["reasons"][0]["message"],
                    "Ubuntu 26.10 removes libarchive (opening archives)."
                );
                assert_eq!(cached(&shared, "stonking").unwrap()["state"], "blocked");
            }
        }
    }

    #[test]
    fn the_cache_rules() {
        let (o, e, x) = (
            offer("stonking", false),
            json!({"status":"tested","gnomeShell":"50"}),
            ext(&["50"]),
        );
        let i = inputs(&e, &x, &o);
        let now = 10 * VALID_MS;
        let rec = |state: &str, age: u64| json!({"version":1,"codename":"stonking","state":state,"checkedAt":now - age,"inputs":i});
        assert!(valid(&rec("ok", VALID_MS - 1), "stonking", &i, now));
        assert!(!valid(&rec("ok", VALID_MS), "stonking", &i, now), "7 days");
        assert!(valid(&rec("unknown", RETRY_MS - 1), "stonking", &i, now));
        assert!(
            !valid(&rec("unknown", RETRY_MS), "stonking", &i, now),
            "a failed check: 1 hour"
        );
        assert!(
            !valid(&rec("ok", 1), "tumbling", &i, now),
            "another codename"
        );
        let newer = inputs(
            &e,
            &Extension {
                version: Some("9".into()),
                ..x.clone()
            },
            &o,
        );
        assert!(
            !valid(&rec("ok", 1), "stonking", &newer, now),
            "a new extension version"
        );
        let retested = inputs(&json!({"status":"tested","gnomeShell":"51"}), &x, &o);
        assert!(
            !valid(&rec("ok", 1), "stonking", &retested, now),
            "a new releases.json entry"
        );
        assert!(
            codename_ok("stonking")
                && !codename_ok("../x")
                && !codename_ok("")
                && !codename_ok("Stonking")
        );
    }

    #[test]
    fn offline_metered_and_failed_checks_are_unknown_with_a_reason() {
        let f = Fixture::new("compat-unknown");
        offered_fixture(&f, "ok", Some(TESTED), false);
        let shared = shared(&f, "nl");
        let fill_now = || {
            let mut r = json!({});
            fill(&shared, &mut r, offered(&shared).as_ref());
            r
        };
        shared
            .store
            .write(
                store::SNAPSHOT,
                &json!({"network":{"state":"none","online":false,"metered":false}}),
            )
            .unwrap();
        assert_eq!(
            compat(&shared, &json!({}), Lang::Nl).unwrap()["skipped"],
            "offline"
        );
        let r = fill_now();
        assert_eq!(
            (r["state"].clone(), r["reasons"][0]["key"].clone()),
            (json!("unknown"), json!("update.compat.offline"))
        );
        assert_eq!(r["reasons"][0]["message"], "Je bent offline.");
        shared
            .store
            .write(
                store::SNAPSHOT,
                &json!({"network":{"state":"full","online":true,"metered":true}}),
            )
            .unwrap();
        assert_eq!(
            compat(&shared, &json!({}), Lang::Nl).unwrap()["skipped"],
            "metered"
        );
        assert_eq!(fill_now()["reasons"][0]["key"], "update.compat.metered");
        assert_eq!(
            shared.setup.tools.runs(),
            0,
            "nothing ran while offline or metered"
        );
        // apt fails (the stub's update exits 100): unknown with update.compat.apt, retried only with refresh
        shared
            .store
            .write(
                store::SNAPSHOT,
                &json!({"network":{"state":"full","online":true,"metered":false}}),
            )
            .unwrap();
        script(
            &f.tools.join("apt-get"),
            &format!(
                "echo \"apt-get $*\" >> '{}'\necho 'Err:1 file:/a stonking InRelease'\nexit 100\n",
                f.log.display()
            ),
        );
        assert_eq!(
            compat(&shared, &json!({}), Lang::Nl).unwrap(),
            json!({"started":true})
        );
        finished(&shared);
        let r = fill_now();
        assert_eq!(
            (r["state"].clone(), r["reasons"][0]["key"].clone()),
            (json!("unknown"), json!("update.compat.apt"))
        );
        assert_eq!(
            r["reasons"][0]["message"],
            "Het pakketoverzicht van Ubuntu 26.10 kon niet worden geladen."
        );
        assert_eq!(
            compat(&shared, &json!({}), Lang::Nl).unwrap()["skipped"],
            "recent"
        );
        assert_eq!(
            compat(&shared, &json!({"refresh":true}), Lang::Nl).unwrap(),
            json!({"started":true})
        );
        finished(&shared);
        assert!(
            !f.base
                .join("cache/update")
                .join(TREE)
                .join("stonking")
                .exists()
        );
    }

    #[test]
    fn the_card_fields_without_an_offer() {
        let f = Fixture::new("compat-none");
        let shared = shared(&f, "nl");
        let mut release = json!({"state":"none"});
        fill(&shared, &mut release, None);
        assert_eq!(release["state"], "none");
        assert_eq!(release["verdict"], "none");
        assert_eq!(release["upgraderNl"], false);
        write(
            &f.root.join("usr/share/locale-langpack").join(UPGRADER_MO),
            "",
        );
        fill(&shared, &mut release, None);
        assert_eq!(release["upgraderNl"], true);
        assert_eq!(
            compat(&shared, &json!({}), Lang::Nl).unwrap()["skipped"],
            "notOffered"
        );
    }

    /// D-G27 (tests/update/test_compat.sh only, inside its bwrap layer with --unshare-net and a private /run): the
    /// real apt-get/apt-cache/apt-config, through the tool table's wrappers, against the signed file: archive of
    /// `W11_COMPAT_CASE`. Prints the record and writes the raw outputs to `W11_COMPAT_OUT`.
    #[test]
    #[ignore = "GW6 D-G27 test: only tests/update/test_compat.sh inside its bwrap layer"]
    fn apt_tree_against_the_fixture_archive() {
        let case = std::env::var("W11_COMPAT_CASE").expect("W11_COMPAT_CASE");
        let out = PathBuf::from(std::env::var("W11_COMPAT_OUT").expect("W11_COMPAT_OUT"));
        assert!(
            std::env::var_os("WIN11_TEST_HOST_NETNS").is_some_and(|host| {
                fs::read_link("/proc/self/ns/net").is_ok_and(|ours| ours.as_os_str() != host)
            }),
            "not inside a network namespace of its own"
        );
        assert!(
            !Path::new("/run/dbus/system_bus_socket").exists(),
            "the system bus is visible"
        );
        let setup = crate::services::update::paths::process_setup().expect("the seams");
        assert!(
            setup.test && setup.paths.fixture,
            "test mode with a fixture root"
        );
        // the service's own directories (its events socket, settings, the update cache with the tree) in the layer's
        // private tmpfs TMPDIR: never a socket in the worktree
        let base = std::env::temp_dir().join(format!("compat-{case}"));
        let events = crate::events::Events::new(&base.join("events")).unwrap();
        let config = base.join("config");
        fs::create_dir_all(&config).unwrap();
        let ctx = crate::services::ServiceContext::new(
            events,
            config,
            base.join("cache"),
            base.clone(),
            "settings".into(),
            false,
        )
        .unwrap();
        let store = store::Store::open(ctx.cache_dir.join("update")).unwrap();
        let shared = Shared {
            ctx,
            setup,
            store,
            stats: Stats::default(),
            rev: AtomicU64::new(0),
        };
        let offer = offered(&shared).expect("the fixture offers a release");
        let releases = releases(&shared.setup.paths);
        let (entry, ext) = (
            entry(&releases, &offer.codename),
            extension(&shared.setup.paths, &releases),
        );
        if case == "cheap" {
            // the embedded releases.json (stonking untested): nothing is built, no program starts
            let answer = compat(&shared, &json!({}), Lang::En).unwrap();
            let mut release = json!({});
            fill(&shared, &mut release, Some(&offer));
            println!(
                "\ncheap: update.release.compat answered {answer}, {} programs started",
                shared.setup.tools.runs()
            );
            println!(
                "\nCOMPAT-RESULT {}",
                json!({"case":case,"record":{"answer":answer},"release":release})
            );
            assert_eq!(answer, json!({"started":false,"skipped":"hold"}));
            assert_eq!(shared.setup.tools.runs(), 0);
            assert!(!tree_dir(&shared, &offer.codename).exists());
            return;
        }
        assert_eq!(
            cheap(&entry, &ext),
            Cheap::Full,
            "the fixture's releases.json marks it tested"
        );
        let (record, raw) = run_full(&shared, &offer, &entry, &ext, store::now_ms());
        for (name, text) in [
            ("dpkg-query.txt", &raw.dpkg),
            ("apt-config-system.txt", &raw.system_dump),
            ("apt-config-tree.txt", &raw.tree_dump),
            ("update.txt", &raw.update),
            ("policy.txt", &raw.policy),
            ("simulate.txt", &raw.simulate),
            ("apt.conf", &raw.conf),
            ("release.sources", &raw.sources),
        ] {
            fs::write(out.join(name), text).unwrap();
        }
        let _ = shared.store.write(&cache_name(&offer.codename), &record);
        let mut release = json!({});
        fill(&shared, &mut release, Some(&offer));
        println!(
            "\nCOMPAT-RESULT {}",
            json!({"case":case,"record":record,"release":release})
        );
        assert!(
            !tree_dir(&shared, &offer.codename).exists(),
            "the tree is deleted"
        );
    }
}
