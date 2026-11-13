//! Restart detection (design §4.8): `/run/reboot-required`, PackageKit's `RequireRestart`, and a
//! needrestart-style scan of `(deleted)` mappings in our processes, their WebKit children and gnome-shell.
//! Files only (the file watch [`Watch`] is read on the pk thread, `pk.rs`).
use super::paths::{GNOME_MAPS, Paths, SHELL_MAPS, WEBKIT_EXE_DIR};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    ffi::{CString, OsString},
    fs,
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
        unix::ffi::{OsStrExt, OsStringExt},
    },
    path::{Path, PathBuf},
    time::{Duration, UNIX_EPOCH},
};

/// The watch on `/run/reboot-required` and apt's `history.log` (design §4.8): one non-blocking inotify
/// descriptor on their parent directories, polled by a `glib` fd source on the pk context. Not a GIO
/// `FileMonitor`: `gio::File::for_path` starts GVfs and its session-bus connection (review F9), and GIO's local
/// VFS singleton (`gio::Vfs::local()`) failed its type check and crashed once when the whole lib suite shared
/// the process (fix round, `notes/claude/wu/gw2.md` §8). inotify is what GIO's local monitor uses underneath.
#[derive(Debug)]
pub(crate) struct Watch {
    fd: OwnedFd,
    /// (watch descriptor, file name in that directory)
    names: Vec<(i32, OsString)>,
}
impl Watch {
    /// None: no directory could be watched (all missing, or no inotify).
    pub(crate) fn new(files: &[PathBuf]) -> Option<Self> {
        let raw = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        if raw < 0 {
            return None;
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let mask = libc::IN_CREATE | libc::IN_CLOSE_WRITE | libc::IN_DELETE | libc::IN_MOVED_TO | libc::IN_MOVED_FROM;
        let mut names = Vec::new();
        for file in files {
            let (Some(dir), Some(name)) = (file.parent(), file.file_name()) else { continue };
            let Ok(dir) = CString::new(dir.as_os_str().as_bytes()) else { continue };
            let wd = unsafe { libc::inotify_add_watch(fd.as_raw_fd(), dir.as_ptr(), mask) };
            if wd >= 0 {
                names.push((wd, name.to_owned()));
            }
        }
        (!names.is_empty()).then_some(Self { fd, names })
    }
    pub(crate) fn fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
    /// Drain the queued events: true when one of them names a watched file.
    pub(crate) fn changed(&self) -> bool {
        const HEADER: usize = std::mem::size_of::<libc::inotify_event>();
        let mut buffer = [0u8; 4096];
        let mut hit = false;
        loop {
            let n = unsafe { libc::read(self.fd.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len()) };
            if n <= 0 {
                return hit;
            }
            let n = n as usize;
            let mut at = 0;
            while at + HEADER <= n {
                let field = |offset: usize| u32::from_ne_bytes([buffer[at + offset], buffer[at + offset + 1], buffer[at + offset + 2], buffer[at + offset + 3]]);
                let wd = field(0) as i32;
                let len = field(12) as usize;
                let end = (at + HEADER + len).min(n);
                let name: Vec<u8> = buffer[at + HEADER..end].iter().copied().take_while(|b| *b != 0).collect();
                let name = OsString::from_vec(name);
                hit |= self.names.iter().any(|(w, file)| *w == wd && *file == name);
                at = end;
            }
        }
    }
}

// WU-FIX (GX1 review n5): a Win11 process that deferred its `shell.restartAll` exec (its own job, Explorer's file
// operation) restarts later than the origin Settings, whose new owner engine scanned at its start and still found
// that process's replaced mappings. That process sends `shell.restartResumed` right before its exec; the owner then
// scans again a few times ([`RESCAN_AFTER_RESUMED`], pk.rs `rescan_after_restart`), so the Win11 restart InfoBar
// goes away once the last process runs the new build.
/// The event a receiver sends when its deferred restart runs after all (`services/shell/restart.rs`).
pub(crate) const RESUMED_EVENT: &str = "shell.restartResumed";
/// The owner's rescans after that event, each delay after the previous one: the exec, the new image's start and the
/// exit of the old image's WebKit children take a moment (Explorer first closes its windows, up to 3 s for a page
/// that hangs). They stop once a scan finds no replaced component of ours.
pub(crate) const RESCAN_AFTER_RESUMED: [Duration; 4] =
    [Duration::from_millis(1500), Duration::from_secs(2), Duration::from_secs(4), Duration::from_secs(8)];
/// A `shell.restartResumed` of another process (`me` is the owner itself: its own exec ends its engine anyway).
pub(crate) fn resumed_elsewhere(data: &Value, me: u32) -> bool {
    data["pid"].as_u64().is_some_and(|pid| pid != u64::from(me))
}
// WU-FIX end

/// At most this many processes are scanned, and at most this much of one maps file is read.
const MAX_PROCESSES: usize = 30;
const MAX_MAPS: u64 = 4 << 20;
const DELETED: &str = " (deleted)";
/// Mappings that are replaced all the time and never mean a restart (today's noise, design §4.8).
const NOISE: [&str; 9] = ["memfd:", "/dev/shm/", "/SYSV", "dconf", "gschemas.compiled", ".cache", "gio/modules/", "gvfs", "libpcre2"];
/// Watch-list packages for the history fallback (design §4.8), by the page's component code (`paths::SHELL_MAPS`).
const HISTORY_PACKAGES: [(&str, &str); 8] = [
    ("libwebkit2gtk-4.1-0", "webkit"),
    ("libjavascriptcoregtk-4.1-0", "jsc"),
    ("libgtk-3-0t64", "gtk"),
    ("libsoup-3.0-0", "soup"),
    ("libglib2.0-0t64", "glib"),
    ("libgdk-pixbuf-2.0-0", "pixbuf"),
    ("libcairo2", "cairo"),
    ("libarchive13t64", "archive"),
];

/// `/run/reboot-required` and its `.pkgs` list.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Reboot {
    pub required: bool,
    pub pkgs: Vec<String>,
    pub since: Option<u64>,
}
pub(crate) fn reboot(paths: &Paths) -> Reboot {
    let Ok(meta) = fs::metadata(&paths.reboot_required) else {
        return Reboot::default();
    };
    let since = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64);
    let text = read_limited(&paths.reboot_required_pkgs, 64 << 10).unwrap_or_default();
    Reboot { required: true, pkgs: parse_pkgs(&text), since }
}
/// One package name per line, unique, in order, at most 200.
pub(crate) fn parse_pkgs(text: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && super::model::valid_name(l))
        .filter(|l| seen.insert(l.to_string()))
        .take(200)
        .map(str::to_owned)
        .collect()
}
/// A `RequireRestart` record counts only in the boot it was seen in, and only for a reboot or a new session.
pub(crate) fn require_restart_applies(record: &Value, boot_id: Option<&str>) -> bool {
    boot_id.is_some()
        && record["bootId"].as_str() == boot_id
        && record["type"]
            .as_u64()
            .is_some_and(|t| super::model::RESTART_TYPES.contains(&(t as u32)))
}

fn read_limited(path: &Path, max: u64) -> Option<String> {
    let mut bytes = Vec::new();
    fs::File::open(path).ok()?.take(max).read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// The result of one deleted-mapping scan.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Scan {
    /// a replaced gnome-shell or mutter is mapped by the running gnome-shell → hero restartRequired
    pub gnome: bool,
    /// components of ours that were replaced → the Win11 restart InfoBar (design §3.4)
    pub shell_parts: Vec<&'static str>,
}
impl Scan {
    pub(crate) fn json(&self) -> Value {
        json!({"gnome":self.gnome,"shell":!self.shell_parts.is_empty(),"shellParts":self.shell_parts})
    }
}

/// The path column of one `/proc/<pid>/maps` line if the mapping is deleted and not noise.
fn deleted_path(line: &str) -> Option<&str> {
    // address perms offset dev inode, then the path after the inode's padding
    let mut rest = line.strip_suffix(DELETED)?;
    for _ in 0..5 {
        rest = rest.trim_start();
        rest = &rest[rest.find(char::is_whitespace)?..];
    }
    let path = rest.trim();
    (!path.is_empty() && !NOISE.iter().any(|noise| path.contains(noise))).then_some(path)
}
/// Components of the `shell` watch list in a maps text.
pub(crate) fn shell_parts_in(maps: &str) -> BTreeSet<&'static str> {
    maps.lines()
        .filter_map(deleted_path)
        .flat_map(|path| SHELL_MAPS.iter().filter(move |(pattern, _)| path.contains(pattern)).map(|(_, part)| *part))
        .collect()
}
/// Does a maps text hold a replaced gnome-shell/mutter?
pub(crate) fn gnome_in(maps: &str) -> bool {
    maps.lines()
        .filter_map(deleted_path)
        .any(|path| GNOME_MAPS.iter().any(|pattern| path.contains(pattern)))
}

/// One process of the procfs used for scans.
struct Proc {
    pid: u32,
    dir: PathBuf,
    comm: String,
    exe: Option<String>,
    uid: Option<u32>,
    ppid: Option<u32>,
}
fn proc_entry(proc: &Path, pid: u32) -> Option<Proc> {
    let dir = proc.join(pid.to_string());
    let status = read_limited(&dir.join("status"), 64 << 10)?;
    let field = |name: &str| {
        status
            .lines()
            .find_map(|l| l.strip_prefix(name))
            .and_then(|v| v.split_whitespace().next())
            .and_then(|v| v.parse::<u32>().ok())
    };
    Some(Proc {
        pid,
        comm: read_limited(&dir.join("comm"), 256).unwrap_or_default().trim().to_owned(),
        exe: fs::read_link(dir.join("exe")).ok().map(|p| p.to_string_lossy().into_owned()),
        uid: field("Uid:"),
        ppid: field("PPid:"),
        dir,
    })
}
fn pids(proc: &Path) -> Vec<u32> {
    let mut pids: Vec<u32> = fs::read_dir(proc)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()))
                .collect()
        })
        .unwrap_or_default();
    pids.sort_unstable();
    pids
}

/// The scan (design §4.8 step 2): our processes (exe in `own_dir`, named `win11-*`), their direct WebKit
/// children, and gnome-shell (our uid, comm, `org.gnome.Shell@` cgroup). `history` is the fallback for a
/// WebKit child whose maps cannot be read.
pub(crate) fn scan(paths: &Paths, uid: u32, own_dir: &Path, history: &[HistoryBlock]) -> Scan {
    let all: Vec<Proc> = pids(&paths.proc).into_iter().filter_map(|pid| proc_entry(&paths.proc, pid)).collect();
    let mine = |p: &Proc| p.uid == Some(uid);
    let own_exe = |exe: &str| {
        let exe = exe.strip_suffix(DELETED).unwrap_or(exe);
        let path = Path::new(exe);
        path.parent() == Some(own_dir)
            && path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("win11-"))
    };
    let ours: Vec<&Proc> = all.iter().filter(|p| mine(p) && p.exe.as_deref().is_some_and(own_exe)).collect();
    let our_pids: BTreeSet<u32> = ours.iter().map(|p| p.pid).collect();
    let children: Vec<&Proc> = all
        .iter()
        .filter(|p| mine(p) && p.ppid.is_some_and(|pp| our_pids.contains(&pp)) && !our_pids.contains(&p.pid))
        .filter(|p| p.comm.starts_with("WebKit") || p.exe.as_deref().is_some_and(|e| e.starts_with(WEBKIT_EXE_DIR)))
        .collect();
    let gnome_shell: Vec<&Proc> = all
        .iter()
        .filter(|p| mine(p) && p.comm == "gnome-shell")
        .filter(|p| read_limited(&p.dir.join("cgroup"), 64 << 10).is_some_and(|c| c.contains("org.gnome.Shell@")))
        .collect();
    let mut out = Scan::default();
    let mut parts = BTreeSet::new();
    let mut budget = MAX_PROCESSES;
    for p in ours.iter().chain(children.iter()) {
        if budget == 0 {
            break;
        }
        budget -= 1;
        if ours.iter().any(|o| o.pid == p.pid) && p.exe.as_deref().is_some_and(|e| e.ends_with(DELETED)) {
            parts.insert("self");
        }
        match read_limited(&p.dir.join("maps"), MAX_MAPS) {
            Some(maps) if !maps.is_empty() => parts.extend(shell_parts_in(&maps)),
            _ if children.iter().any(|c| c.pid == p.pid) => {
                if let Some(started) = start_ms(paths, p.pid) {
                    parts.extend(history_parts(history, started));
                }
            }
            _ => {}
        }
    }
    for p in gnome_shell.iter().take(budget.max(1)) {
        if read_limited(&p.dir.join("maps"), MAX_MAPS).is_some_and(|maps| gnome_in(&maps)) {
            out.gnome = true;
        }
    }
    out.shell_parts = parts.into_iter().collect();
    out
}

/// A process's start as wall-clock ms: `/proc/<pid>/stat` field 22 (ticks since boot) + `btime` of `/proc/stat`.
fn start_ms(paths: &Paths, pid: u32) -> Option<u64> {
    let ticks: u64 = super::store::process_start_in(&paths.proc, pid)?.parse().ok()?;
    let stat = read_limited(&paths.proc.join("stat"), 1 << 20)?;
    let btime: u64 = stat.lines().find_map(|l| l.strip_prefix("btime "))?.trim().parse().ok()?;
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    let hz = if hz > 0 { hz as u64 } else { 100 };
    Some(btime * 1000 + ticks * 1000 / hz)
}

/// One `Start-Date:` block of `/var/log/apt/history.log` with the packages it upgraded or installed.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HistoryBlock {
    pub start_ms: u64,
    pub packages: Vec<String>,
}
/// The plain `history.log` (rotated `.gz` files are GW4's parser); bounded to 4 MiB.
pub(crate) fn history(paths: &Paths) -> Vec<HistoryBlock> {
    read_limited(&paths.apt_log.join("history.log"), 4 << 20)
        .map(|text| parse_history(&text))
        .unwrap_or_default()
}
pub(crate) fn parse_history(text: &str) -> Vec<HistoryBlock> {
    let mut out = Vec::new();
    let mut current: Option<HistoryBlock> = None;
    for line in text.lines() {
        if let Some(date) = line.strip_prefix("Start-Date:") {
            if let Some(block) = current.take() {
                out.push(block);
            }
            current = local_ms(date.trim()).map(|start_ms| HistoryBlock { start_ms, packages: Vec::new() });
        } else if let Some(block) = current.as_mut() {
            for key in ["Upgrade:", "Install:"] {
                if let Some(list) = line.strip_prefix(key) {
                    block.packages.extend(parse_package_list(list));
                }
            }
        }
    }
    out.extend(current);
    out
}
/// "libsoup-3.0-0:amd64 (3.6.6-1ubuntu0.1, 3.6.6-1ubuntu0.2), zlib1g:amd64 (…)" → names without the arch.
fn parse_package_list(list: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut name = String::new();
    for c in list.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                push_name(&mut out, &name);
                name.clear();
            }
            _ if depth == 0 => name.push(c),
            _ => {}
        }
    }
    push_name(&mut out, &name);
    out
}
fn push_name(out: &mut Vec<String>, raw: &str) {
    let name = raw.trim().split(':').next().unwrap_or("").trim();
    if super::model::valid_name(name) {
        out.push(name.to_owned());
    }
}
/// "2026-11-05  06:53:43" in local time → ms since the epoch.
fn local_ms(text: &str) -> Option<u64> {
    let mut it = text.split_whitespace();
    let date: Vec<i32> = it.next()?.split('-').map(|v| v.parse().ok()).collect::<Option<_>>()?;
    let time: Vec<i32> = it.next()?.split(':').map(|v| v.parse().ok()).collect::<Option<_>>()?;
    if date.len() != 3 || time.len() != 3 {
        return None;
    }
    let dt = glib::DateTime::from_local(date[0], date[1], date[2], time[0], time[1], time[2] as f64).ok()?;
    u64::try_from(dt.to_unix()).ok().map(|s| s * 1000)
}
/// Components that a history block after `started` (ms) replaced.
pub(crate) fn history_parts(history: &[HistoryBlock], started: u64) -> BTreeSet<&'static str> {
    history
        .iter()
        .filter(|b| b.start_ms >= started)
        .flat_map(|b| b.packages.iter())
        .filter_map(|name| HISTORY_PACKAGES.iter().find(|(pkg, _)| pkg == name).map(|(_, part)| *part))
        .collect()
}
