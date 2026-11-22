//! `update.history`: the apt history parser (design §3.6, H's groups "Updates / Ubuntu-versies / Software", C8).
//!
//! * Sources (through the path table, so tests read fixtures under `WIN11_UPDATE_FIXTURE_ROOT`): apt's own
//!   `/var/log/apt/history.log`, `history.log.1` and `history.log.1.gz`–`.4.gz` (one `Start-Date:` block per apt
//!   run), and the logs of Ubuntu's release upgrader, which points `Dir::Log` at `/var/log/dist-upgrade/`
//!   (`DistUpgradeCache._initAptLog`) and moves an earlier run's logs into a `YYYYMMDD-HHMM/` directory.
//!   Software Updater's partial upgrade (`do-partial-upgrade`) runs the same code, so a `dist-upgrade` log is a
//!   release upgrade only when one of the upgrader's own packages moves to another release
//!   (`1:25.10.9 → 1:26.04.14`); any other `dist-upgrade` log is listed like apt's runs.
//! * Entries are per package: `{group, name, version, date, ok, source, cursor}`. `Upgrade:` → `updates`;
//!   `Install:` (not the `automatic` dependencies), `Reinstall:` and `Downgrade:` → `software`; a release upgrade →
//!   `releases` (name `Ubuntu`, version = the release the upgrader moved to, `26.04`). Removals are not listed
//!   (Windows lists installed updates). `ok` is false for a run with an `Error:` line or one that never wrote its
//!   `End-Date:` (a crash, a cut log); the newest run without `End-Date` while apt's lock is held is still running
//!   and left out.
//! * `source`: `ubuntu` (unattended-upgrades), `us` (PackageKit, `Requested-By` our uid, inside one of our job
//!   windows: every `jobs.jsonl` line, and `job.json` until now only while it is unfinished in this boot with a
//!   live owner, else until its last write; PackageKit writes only `packagekit role='…'`), `app` (another
//!   PackageKit caller, aptdaemon, a python-apt client without a command line such as the partial upgrade),
//!   `terminal` (apt, apt-get, aptitude, dpkg); `null` for a release upgrade that was not ours (the log does not
//!   say how it ran) and for a PackageKit run of our uid that ended before `jobs.jsonl`'s 90-day horizon (its
//!   window may have been pruned: ours or an app's).
//! * Paging: newest first, at most 200 entries and 90 days per call, counted from the newest entry before
//!   `before`, then `more`. `date` is the run's `End-Date` (else `Start-Date`) in ms. `cursor` orders every entry
//!   uniquely (`second << 20 | slot`, the slots of one second in log order, below 2^53 so JavaScript keeps it
//!   exact); the page passes the last entry's `cursor` as `before`. A `before` below 2^48 is a time in ms (the
//!   design's form: the entries older than it).
//! * Read-only, no program, no bus: the same in every mode (owner, mirror, reader, dry-run). Bytes that are not
//!   UTF-8 are replaced; a cut gzip file gives what was decompressed; of a file beyond the bounds the newest part
//!   (apt appends) from its first whole run. Parsed files are cached by (size, mtime, inode, time zone); entries
//!   carry codes only, the page renders every text.
use super::{Shared, guard, paths::Paths, store};
use crate::{files::ApiResult, i18n::Lang};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

/// At most this many entries per call (`update.rs` also refuses a larger `limit`).
pub(crate) const MAX_ENTRIES: usize = 200;
/// One call covers at most 90 days, counted from its newest entry.
pub(crate) const WINDOW_MS: u64 = 90 * 86_400_000;
/// `store.rs` drops `jobs.jsonl` lines that finished 90 days before an append (`JOBS_DAYS`).
const JOBS_KEPT_MS: u64 = 90 * 86_400_000;
/// apt's own files, newest first (design §3.6: `.1.gz`–`.4.gz`; a plain `.1` when logrotate delays compression).
const APT_FILES: [&str; 6] =
    ["history.log", "history.log.1", "history.log.1.gz", "history.log.2.gz", "history.log.3.gz", "history.log.4.gz"];
/// Per file: at most this much is read (the end of a plain file), at most this much text is kept of a gzip file
/// (its end), and at most this much is inflated.
const MAX_READ: u64 = 8 << 20;
const MAX_TEXT: usize = 32 << 20;
const MAX_INFLATE: usize = 256 << 20;
/// At most this many earlier release-upgrade directories are read (the newest).
const MAX_RELEASE_DIRS: usize = 16;
/// The release upgrader's own packages: their version names the release (`1:26.10.5`).
const UPGRADER_PACKAGES: [&str; 4] =
    ["ubuntu-release-upgrader-core", "python3-distupgrade", "update-manager-core", "ubuntu-release-upgrader-gtk"];
/// `cursor` = `second << SLOT_BITS | slot` (dates 2000–2200, so a cursor stays below 2^53).
const SLOT_BITS: u32 = 20;
const SLOT_MAX: u64 = (1 << SLOT_BITS) - 1;
/// A `before` from here up is a cursor, below it a time in ms (ms reach 2^48 only in the year 10889; the cursor
/// of 1 Jan 2000 is about 2^49.8).
pub(crate) const CURSOR_MIN: u64 = 1 << 48;

/// One apt run (a `Start-Date:` block).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Run {
    /// `Start-Date` / `End-Date` in ms (None when missing or not a date)
    pub start: Option<u64>,
    pub end: Option<u64>,
    pub commandline: Option<String>,
    /// the uid of `Requested-By: robin (1000)`
    pub uid: Option<u32>,
    /// an `Error:` line
    pub error: bool,
    /// `Upgrade:` → (name, new version)
    pub upgraded: Vec<(String, String)>,
    /// `Install:` without the automatic ones, `Reinstall:`, `Downgrade:` → (name, version)
    pub installed: Vec<(String, String)>,
    /// the release an upgrader package moved to (`(1:25.10.9, 1:26.04.14)` → `26.04`): a release upgrade
    pub release: Option<String>,
}

/// One history entry of `update.history`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Entry {
    pub group: &'static str,
    pub name: String,
    pub version: String,
    pub date: u64,
    pub ok: bool,
    pub source: Option<&'static str>,
    pub cursor: u64,
}
impl Entry {
    pub(crate) fn json(&self) -> Value {
        json!({"group":self.group,"name":self.name,"version":self.version,"date":self.date,"ok":self.ok,
               "source":self.source,"cursor":self.cursor})
    }
}

/// What decides `source` and which run is still running.
pub(crate) struct Context<'a> {
    pub uid: u32,
    /// our jobs: (startedAt, finishedAt) in ms
    pub windows: &'a [(u64, u64)],
    /// a run of ours that ended before this (ms) may have lost its `jobs.jsonl` window
    pub horizon: u64,
    /// apt's lock is held now (asked only when the newest run has no `End-Date`)
    pub apt_busy: &'a dyn Fn() -> bool,
    /// Ubuntu's release upgrader runs now (asked only when a current `dist-upgrade/history.log` exists)
    pub upgrading: &'a dyn Fn() -> bool,
}

/// `{before?:cursor|ms, limit?:≤200}` (checked by `update.rs`) →
/// `{entries:[{group, name, version, date, ok, source, cursor}], more}`.
pub(super) fn history(shared: &Shared, args: &Value, _lang: Lang) -> ApiResult {
    let paths = &shared.setup.paths;
    let before = args["before"].as_u64();
    let limit = args["limit"].as_u64().map_or(MAX_ENTRIES, |n| (n as usize).clamp(1, MAX_ENTRIES));
    let now = store::now_ms();
    let windows = job_windows(&shared.store, paths, now);
    let horizon = jobs_horizon(&shared.store, now);
    let (apt, releases) = read_logs(paths, &glib::TimeZone::local());
    let apt_busy = || guard::apt_lock_held(paths);
    let upgrading = || guard::upgrader_running(paths);
    let context =
        Context { uid: unsafe { libc::getuid() }, windows: &windows, horizon, apt_busy: &apt_busy, upgrading: &upgrading };
    let (entries, more) = page(entries(&apt, &releases, &context), before, limit);
    Ok(json!({"entries":entries.iter().map(Entry::json).collect::<Vec<_>>(),"more":more}))
}

/// Our job windows: every finished job of `jobs.jsonl`, and `job.json` until its `finishedAt`. An unfinished
/// `job.json` lasts until now only while it belongs to this boot and its owner lives (update.rs's own filter);
/// otherwise (a reboot during the download: GW2 never finishes a job of an earlier boot) until its last write.
pub(crate) fn job_windows(store: &store::Store, paths: &Paths, now: u64) -> Vec<(u64, u64)> {
    let mut out: Vec<(u64, u64)> = store
        .jobs()
        .iter()
        .filter_map(|j| Some((j["startedAt"].as_u64()?, j["finishedAt"].as_u64()?)))
        .collect();
    if let Some(job) = store.read_job()
        && let Some(started) = job["startedAt"].as_u64()
    {
        let end = job["finishedAt"].as_u64().unwrap_or_else(|| {
            let boot = store::boot_id(paths);
            let live =
                boot.is_some() && job["bootId"].as_str() == boot.as_deref() && store::owner_alive(&job["owner"]);
            if live { now } else { store.modified_ms(store::JOB).unwrap_or(started).min(now) }
        });
        out.push((started, end.max(started)));
    }
    out
}

/// The time before which a run of ours may have lost its window: 90 days before the last `jobs.jsonl` append
/// (its mtime; each append drops what finished 90 days earlier), at the latest 90 days before now.
pub(crate) fn jobs_horizon(store: &store::Store, now: u64) -> u64 {
    store.modified_ms(store::JOBS).unwrap_or(now).min(now).saturating_sub(JOBS_KEPT_MS)
}

/// One `dist-upgrade` log; `current` = the top-level `dist-upgrade/history.log` (maybe still being written).
pub(crate) struct Release {
    pub current: bool,
    pub runs: Arc<Vec<Run>>,
}

/// apt's runs (oldest file first, so in log order) and the `dist-upgrade` logs (oldest first, the current last).
pub(crate) fn read_logs(paths: &Paths, tz: &glib::TimeZone) -> (Vec<Run>, Vec<Release>) {
    let mut apt = Vec::new();
    for name in APT_FILES.iter().rev() {
        apt.extend(read_runs(&paths.apt_log.join(name), tz).iter().cloned());
    }
    let mut releases = Vec::new();
    let mut dirs: Vec<PathBuf> = fs::read_dir(&paths.dist_upgrade_log)
        .map(|it| it.flatten().filter(|e| backup_dir_name(&e.file_name().to_string_lossy())).map(|e| e.path()).collect())
        .unwrap_or_default();
    dirs.sort();
    let skip = dirs.len().saturating_sub(MAX_RELEASE_DIRS);
    for dir in dirs.into_iter().skip(skip) {
        let file = dir.join("history.log");
        if file.is_file() {
            releases.push(Release { current: false, runs: read_runs(&file, tz) });
        }
    }
    let current = paths.dist_upgrade_log.join("history.log");
    if current.is_file() {
        releases.push(Release { current: true, runs: read_runs(&current, tz) });
    }
    (apt, releases)
}

/// `20260820-1012`: the directory `DistUpgradeMain.setup_logging` moves an earlier run's logs into.
fn backup_dir_name(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() == 13 && b.iter().enumerate().all(|(i, c)| if i == 8 { *c == b'-' } else { c.is_ascii_digit() })
}

type CacheKey = (u64, i64, i64, u64, String);
static CACHE: Mutex<Vec<(PathBuf, CacheKey, Arc<Vec<Run>>)>> = Mutex::new(Vec::new());
const CACHE_FILES: usize = 32;

/// The runs of one file (plain or `.gz`), cached by size, mtime, inode and time zone. Missing or unreadable: none.
fn read_runs(path: &Path, tz: &glib::TimeZone) -> Arc<Vec<Run>> {
    let Ok(meta) = fs::metadata(path) else { return Arc::default() };
    if !meta.is_file() {
        return Arc::default();
    }
    let key: CacheKey = (meta.len(), meta.mtime(), meta.mtime_nsec(), meta.ino(), tz.identifier().to_string());
    if let Some((_, _, runs)) =
        CACHE.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(p, k, _)| p == path && *k == key)
    {
        return runs.clone();
    }
    let bytes = if path.extension().is_some_and(|e| e == "gz") {
        let mut raw = Vec::new();
        if fs::File::open(path).and_then(|f| f.take(MAX_READ).read_to_end(&mut raw)).is_err() {
            return Arc::default();
        }
        gunzip_tail(&raw, MAX_TEXT)
    } else {
        let Ok(bytes) = read_tail(path, MAX_READ) else { return Arc::default() };
        bytes
    };
    let runs = Arc::new(parse(&bytes, tz));
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|(p, _, _)| p != path);
    if cache.len() >= CACHE_FILES {
        cache.remove(0);
    }
    cache.push((path.to_path_buf(), key, runs.clone()));
    runs
}

/// A plain file whole, or its last `max` bytes from the first run that starts in them (apt appends: the newest
/// runs are at the end, review m4).
pub(crate) fn read_tail(path: &Path, max: u64) -> std::io::Result<Vec<u8>> {
    let mut file = fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len <= max {
        let mut bytes = Vec::new();
        file.take(max).read_to_end(&mut bytes)?;
        return Ok(bytes);
    }
    // one byte more: whether the tail starts at a line start
    file.seek(SeekFrom::Start(len - max - 1))?;
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes)?;
    Ok(from_first_run(&bytes).to_vec())
}
/// The text from the first `Start-Date:` that follows a newline (what comes before it belongs to a cut run).
fn from_first_run(bytes: &[u8]) -> &[u8] {
    const MARK: &[u8] = b"\nStart-Date:";
    bytes.windows(MARK.len()).position(|w| w == MARK).map_or(&[][..], |at| &bytes[at + 1..])
}

/// Decompress a gzip file (one or more members) in process with GIO's zlib converter (no program); a cut or
/// damaged file gives what came out before the damage. At most [`MAX_INFLATE`] bytes are inflated and the last
/// `max` are kept, from the first run that starts in them when the front was dropped.
pub(crate) fn gunzip_tail(bytes: &[u8], max: usize) -> Vec<u8> {
    use gio::prelude::*;
    let z = gio::ZlibDecompressor::new(gio::ZlibCompressorFormat::Gzip);
    let (mut out, mut dropped, mut total) = (Vec::new(), false, 0usize);
    // keeps one byte more than `max`: whether the tail starts at a line start
    let keep = |out: &mut Vec<u8>, dropped: &mut bool| {
        if out.len() > max {
            out.drain(..out.len() - max - 1);
            *dropped = true;
        }
    };
    let mut input = bytes;
    let mut buf = vec![0u8; 1 << 16];
    while total < MAX_INFLATE && !input.is_empty() {
        match z.convert(input, &mut buf[..], gio::ConverterFlags::INPUT_AT_END) {
            Ok((result, read, written)) => {
                out.extend_from_slice(&buf[..written]);
                total += written;
                input = &input[read..];
                if out.len() > max.saturating_mul(2).saturating_add(1 << 16) {
                    keep(&mut out, &mut dropped);
                }
                if result == gio::ConverterResult::Finished {
                    // another member may follow (`cat a.gz b.gz`)
                    z.reset();
                } else if read == 0 && written == 0 {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    keep(&mut out, &mut dropped);
    if dropped { from_first_run(&out).to_vec() } else { out }
}

/// Parse apt's history format. Lines that are not `Key: value`, unknown keys and runs without a valid
/// `Start-Date` are skipped; text that is not UTF-8 is replaced.
pub(crate) fn parse(bytes: &[u8], tz: &glib::TimeZone) -> Vec<Run> {
    let mut out = Vec::new();
    let mut current: Option<Run> = None;
    for raw in bytes.split(|b| *b == b'\n') {
        let line = String::from_utf8_lossy(raw);
        let Some((key, value)) = line.trim_end_matches('\r').split_once(':') else { continue };
        let value = value.trim();
        if key == "Start-Date" {
            out.extend(current.take());
            current = Some(Run { start: local_ms(value, tz), ..Run::default() });
            continue;
        }
        let Some(run) = current.as_mut() else { continue };
        match key {
            "End-Date" => run.end = local_ms(value, tz),
            "Commandline" => run.commandline = Some(value.chars().take(512).collect()),
            "Requested-By" => run.uid = requested_uid(value),
            "Error" => run.error = true,
            "Upgrade" => {
                if run.release.is_none() {
                    run.release = release_change(value);
                }
                push_all(&mut run.upgraded, packages(value, Pick::New));
            }
            "Downgrade" => push_all(&mut run.installed, packages(value, Pick::New)),
            "Install" | "Reinstall" => push_all(&mut run.installed, packages(value, Pick::Installed)),
            _ => {}
        }
    }
    out.extend(current);
    out.retain(|r| r.start.is_some());
    out
}
fn push_all(list: &mut Vec<(String, String)>, items: Vec<(String, String)>) {
    for item in items {
        if !list.contains(&item) {
            list.push(item);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Pick {
    /// `(old, new)`: the new version
    New,
    /// `(version)` or `(version, automatic)`: the version; automatic ones are left out
    Installed,
}
/// "libsoup-3.0-0:amd64 (3.6.6-1ubuntu0.1, 3.6.6-1ubuntu0.2), zlib1g:i386 (…)" → (name without the architecture,
/// the parts inside the brackets). An item cut off before its `)` ends the list.
fn items(list: &str) -> Vec<(&str, Vec<&str>)> {
    let mut out = Vec::new();
    let mut rest = list;
    loop {
        let item = rest.trim_start_matches([',', ' ']);
        let Some(open) = item.find('(') else { break };
        let Some(close) = item[open..].find(')').map(|c| open + c) else { break };
        let name = item[..open].trim().split(':').next().unwrap_or("").trim();
        out.push((name, item[open + 1..close].split(',').map(str::trim).collect()));
        rest = &item[close + 1..];
    }
    out
}
/// The (name, version) pairs of a list, each once (multi-arch), names and versions checked.
fn packages(list: &str, pick: Pick) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (name, parts) in items(list) {
        let version = match pick {
            Pick::New => parts.get(1).or(parts.first()),
            Pick::Installed if parts.iter().skip(1).any(|p| *p == "automatic") => None,
            Pick::Installed => parts.first(),
        };
        if let Some(version) = version
            && super::model::valid_name(name)
            && valid_version(version)
        {
            let item = (name.to_owned(), (*version).to_owned());
            if !out.contains(&item) {
                out.push(item);
            }
        }
    }
    out
}
/// The release an upgrader package of an `Upgrade:` list moves to, when its old version names another release
/// (review m2: Software Updater's partial upgrade writes the same logs, within one release).
fn release_change(list: &str) -> Option<String> {
    items(list).into_iter().find_map(|(name, parts)| {
        let (old, new) = (release_of(parts.first()?), release_of(parts.get(1)?));
        (UPGRADER_PACKAGES.contains(&name) && !old.is_empty() && !new.is_empty() && old != new).then_some(new)
    })
}
fn valid_version(v: &str) -> bool {
    (1..=128).contains(&v.len()) && v.bytes().all(|c| c.is_ascii_alphanumeric() || b".+~:-".contains(&c))
}
/// "robin (1000)" → 1000.
fn requested_uid(value: &str) -> Option<u32> {
    let open = value.rfind('(')?;
    let close = value[open..].find(')')? + open;
    value[open + 1..close].trim().parse().ok()
}
/// "2026-11-05  06:53:43" in `tz` → ms since the epoch (years 2000–2200: apt's history is younger, and the
/// cursor needs the bound).
pub(crate) fn local_ms(text: &str, tz: &glib::TimeZone) -> Option<u64> {
    let mut it = text.split_whitespace();
    let date: Vec<i32> = it.next()?.split('-').map(|v| v.parse().ok()).collect::<Option<_>>()?;
    let time: Vec<i32> = it.next()?.split(':').map(|v| v.parse().ok()).collect::<Option<_>>()?;
    if date.len() != 3 || time.len() != 3 || !(2000..=2200).contains(&date[0]) {
        return None;
    }
    let dt = glib::DateTime::new(tz, date[0], date[1], date[2], time[0], time[1], f64::from(time[2])).ok()?;
    u64::try_from(dt.to_unix()).ok().map(|s| s * 1000)
}

/// Who ran it (module doc).
fn source_of(run: &Run, context: &Context) -> Option<&'static str> {
    let commandline = run.commandline.as_deref().unwrap_or("").trim();
    let program = commandline.split_whitespace().next().unwrap_or("");
    match program.rsplit('/').next().unwrap_or("") {
        "unattended-upgrade" | "unattended-upgrades" => Some("ubuntu"),
        "packagekit" if run.uid == Some(context.uid) => {
            if ours(run, context.windows) {
                Some("us")
            } else if run.end.or(run.start).is_some_and(|t| t < context.horizon) {
                // older than jobs.jsonl keeps our windows (review m3): ours or an app's
                None
            } else {
                Some("app")
            }
        }
        "apt" | "apt-get" | "aptitude" | "dpkg" => Some("terminal"),
        _ => Some("app"),
    }
}
/// The run lies inside one of our job windows (`Start-Date`/`End-Date` have whole seconds; packagekitd
/// finishes a few seconds after dpkg).
fn ours(run: &Run, windows: &[(u64, u64)]) -> bool {
    let Some(start) = run.start else { return false };
    let end = run.end.unwrap_or(start);
    windows.iter().any(|(from, to)| start + 2000 >= *from && end <= to.saturating_add(5000))
}

/// The entries of every run and release upgrade, newest first (by `cursor`).
pub(crate) fn entries(apt: &[Run], releases: &[Release], context: &Context) -> Vec<Entry> {
    // apt's runs plus the dist-upgrade logs that are no release upgrade (Software Updater's partial upgrades)
    let mut runs: Vec<&Run> = apt.iter().collect();
    // (date, start, the entries in display order); the cursors are set below
    let mut blocks: Vec<(u64, u64, Vec<Entry>)> = Vec::new();
    let entry = |group, name: &str, version: &str, date, ok, source| Entry {
        group,
        name: name.to_owned(),
        version: version.to_owned(),
        date,
        ok,
        source,
        cursor: 0,
    };
    for release in releases {
        // the current log is still being written while the upgrader runs (also a partial upgrade's: apt's lock)
        if release.runs.is_empty() || (release.current && (context.upgrading)()) {
            continue;
        }
        let Some(version) = release.runs.iter().find_map(|r| r.release.clone()) else {
            runs.extend(release.runs.iter());
            continue;
        };
        let Some(date) = release.runs.iter().filter_map(|r| r.end.or(r.start)).max() else { continue };
        let start = release.runs.iter().filter_map(|r| r.start).min().unwrap_or(date);
        let ok = release.runs.iter().all(|r| !r.error && r.end.is_some());
        let source = release.runs.iter().any(|r| r.uid == Some(context.uid) && ours(r, context.windows)).then_some("us");
        blocks.push((date, start, vec![entry("releases", "Ubuntu", &version, date, ok, source)]));
    }
    // the newest run without End-Date may still be running (apt's lock): not listed yet
    let newest = runs.iter().enumerate().max_by_key(|(_, r)| r.start).map(|(i, _)| i);
    let running = newest.filter(|i| runs[*i].end.is_none() && (context.apt_busy)());
    for (index, run) in runs.iter().enumerate() {
        let Some(date) = run.end.or(run.start).filter(|_| Some(index) != running) else { continue };
        let ok = !run.error && run.end.is_some();
        let source = source_of(run, context);
        let list: Vec<Entry> = run
            .upgraded
            .iter()
            .map(|p| ("updates", p))
            .chain(run.installed.iter().map(|p| ("software", p)))
            .map(|(group, (name, version))| entry(group, name, version, date, ok, source))
            .collect();
        if !list.is_empty() {
            blocks.push((date, run.start.unwrap_or(date), list));
        }
    }
    // chronological (the stable sort keeps the log order of equal times), then the slots of each second: a later
    // block gets the higher slots, inside a block the first entry the highest (review m1: every cursor unique)
    blocks.sort_by_key(|(date, start, _)| (date / 1000, *start));
    let mut out = Vec::new();
    let (mut second, mut used) = (u64::MAX, 0u64);
    for (date, _, list) in blocks {
        if date / 1000 != second {
            (second, used) = (date / 1000, 0);
        }
        let n = list.len() as u64;
        for (i, mut e) in list.into_iter().enumerate() {
            e.cursor = (second << SLOT_BITS) | (used + n - 1 - i as u64).min(SLOT_MAX);
            out.push(e);
        }
        used += n;
    }
    out.sort_by(|a, b| b.cursor.cmp(&a.cursor));
    out
}
/// "1:26.10.5" → "26.10" (the upgrader's version follows its release); anything else → "".
fn release_of(version: &str) -> String {
    let upstream = version.split_once(':').map_or(version, |(_, v)| v);
    let mut parts = upstream.split(|c: char| !c.is_ascii_digit());
    match (parts.next(), parts.next()) {
        (Some(y), Some(m)) if y.len() == 2 && m.len() == 2 => format!("{y}.{m}"),
        _ => String::new(),
    }
}

/// Newest first: the entries before `before` (a cursor, or a time in ms below [`CURSOR_MIN`]), at most `limit`
/// and 90 days from the newest of them; `more` when older entries remain.
pub(crate) fn page(entries: Vec<Entry>, before: Option<u64>, limit: usize) -> (Vec<Entry>, bool) {
    let older: Vec<Entry> = entries
        .into_iter()
        .filter(|e| match before {
            None => true,
            Some(b) if b >= CURSOR_MIN => e.cursor < b,
            Some(b) => e.date < b,
        })
        .collect();
    let Some(first) = older.first() else { return (Vec::new(), false) };
    let floor = first.date.saturating_sub(WINDOW_MS);
    let take = older.iter().take_while(|e| e.date >= floor).count().min(limit);
    let more = older.len() > take;
    (older.into_iter().take(take).collect(), more)
}

// ------------------------------------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::update::{
        Shared, Stats,
        paths::{self, Env, VAR_FIXTURE_ROOT, VAR_TOOL_DIR},
        tests::{Fixture, write},
    };
    use std::{
        sync::atomic::AtomicU64,
        time::{Duration, UNIX_EPOCH},
    };

    /// `tests/update/fixtures/history/` (written by its `make.py`).
    fn fixtures() -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/update/fixtures/history");
        assert!(dir.join("var/log/apt/history.log").is_file(), "run tests/update/fixtures/history/make.py");
        dir
    }
    fn utc() -> glib::TimeZone {
        glib::TimeZone::utc()
    }
    fn at(text: &str) -> u64 {
        local_ms(text, &utc()).unwrap()
    }
    fn never() -> bool {
        false
    }
    fn always() -> bool {
        true
    }
    fn context<'a>(windows: &'a [(u64, u64)], busy: &'a dyn Fn() -> bool) -> Context<'a> {
        Context { uid: 1000, windows, horizon: 0, apt_busy: busy, upgrading: &never }
    }
    fn copy_tree(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap().flatten() {
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    /// The fixture's apt runs and dist-upgrade logs, in UTC.
    fn fixture_logs() -> (Vec<Run>, Vec<Release>) {
        let dir = fixtures();
        let mut paths = paths::resolve(&Env::fixture(&[], 1000)).unwrap().paths;
        paths.apt_log = dir.join("var/log/apt");
        paths.dist_upgrade_log = dir.join("var/log/dist-upgrade");
        read_logs(&paths, &utc())
    }
    fn names(entries: &[Entry]) -> Vec<String> {
        entries.iter().map(|e| format!("{} {} {}", e.group, e.name, e.version)).collect()
    }
    fn shared_in(f: &Fixture, kind: &str, dry: bool, root: bool) -> Shared {
        let mut vars = vec![(VAR_TOOL_DIR, f.tools.display().to_string())];
        if root {
            vars.push((VAR_FIXTURE_ROOT, f.root.display().to_string()));
        }
        let pairs: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let setup = paths::resolve(&Env::fixture(&pairs, unsafe { libc::geteuid() })).unwrap();
        let ctx = f.ctx(kind, dry, "nl");
        let store = store::Store::open(ctx.cache_dir.join("update")).unwrap();
        Shared { ctx, setup, store, stats: Stats::default(), rev: AtomicU64::new(0) }
    }
    /// A finished run of `n` upgrades `{prefix}0…` ending at `end` (ms), started by `commandline`.
    fn run_of(prefix: &str, n: usize, start: u64, end: Option<u64>, commandline: &str) -> Run {
        Run {
            start: Some(start),
            end,
            commandline: Some(commandline.into()),
            uid: Some(1000),
            upgraded: (0..n).map(|i| (format!("{prefix}{i}"), "1.0-1ubuntu0.1".to_owned())).collect(),
            ..Run::default()
        }
    }
    /// Every entry, walking the pages at `limit` from the start (the page's way: `before` = the last cursor).
    fn walk(all: &[Entry], limit: usize) -> Vec<Entry> {
        let (mut seen, mut before) = (Vec::new(), None);
        loop {
            let (part, more) = page(all.to_vec(), before, limit);
            assert!(!part.is_empty() && part.len() <= limit);
            seen.extend(part.iter().cloned());
            before = part.last().map(|e| e.cursor);
            if !more {
                return seen;
            }
        }
    }

    #[test]
    fn a_run_is_parsed_with_its_lists_dates_uid_and_error() {
        let text = "Start-Date: 2026-11-03  19:20:11\nCommandline: apt install vim\nRequested-By: robin (1000)\n\
            Install: vim:amd64 (2:9.1.0967-1ubuntu4), vim-runtime:amd64 (2:9.1.0967-1ubuntu4, automatic)\n\
            Upgrade: vim-common:amd64 (2:9.1.0967-1ubuntu3, 2:9.1.0967-1ubuntu4), vim-common:i386 (2:9.1.0967-1ubuntu3, 2:9.1.0967-1ubuntu4)\n\
            Remove: nano:amd64 (8.4-1)\nError: Sub-process dpkg returned an error code (1)\nEnd-Date: 2026-11-03  19:20:30\n";
        let runs = parse(text.as_bytes(), &utc());
        assert_eq!(runs.len(), 1);
        let run = &runs[0];
        assert_eq!(run.start, Some(at("2026-11-03 19:20:11")));
        assert_eq!(run.end, Some(at("2026-11-03 19:20:30")));
        assert_eq!(run.commandline.as_deref(), Some("apt install vim"));
        assert_eq!(run.uid, Some(1000));
        assert!(run.error);
        assert_eq!(run.installed, [("vim".to_owned(), "2:9.1.0967-1ubuntu4".to_owned())], "no automatic dependency");
        assert_eq!(run.upgraded, [("vim-common".to_owned(), "2:9.1.0967-1ubuntu4".to_owned())], "multi-arch once");
        assert_eq!(run.release, None);
        assert_eq!(local_ms("2026-11-05  06:53:43", &utc()), Some(1_793_861_623_000));
        for bad in ["", "2026-11-05", "2026-13-05 06:53:43", "2026-11-05 06:61:43", "x-11-05 06:53:43", "1969-12-31 23:59:59",
                    "1999-12-31 23:59:59", "2201-01-01 00:00:00"] {
            assert_eq!(local_ms(bad, &utc()), None, "{bad}");
        }
        assert_eq!(requested_uid("robin (1000)"), Some(1000));
        assert_eq!(requested_uid("robin"), None);
        // an upgrader package that moves to another release marks a release upgrade; within one release it does not
        let release = |list: &str| parse(format!("Start-Date: 2026-08-20  10:15:00\nUpgrade: {list}\n").as_bytes(), &utc())[0].release.clone();
        assert_eq!(release("base-files:amd64 (14ubuntu1, 14ubuntu5), ubuntu-release-upgrader-core:all (1:25.10.9, 1:26.04.14)").as_deref(), Some("26.04"));
        assert_eq!(release("update-manager-core:all (1:26.04.3, 1:26.04.4), ubuntu-release-upgrader-core:all (1:26.04.14, 1:26.04.15)"), None);
        assert_eq!(release("gnome-shell:amd64 (49.0-1ubuntu1, 50.1-0ubuntu1)"), None, "not an upgrader package");
        assert_eq!(release("python3-distupgrade:all (0.1, 1:26.04.14)"), None, "an old version that names no release");
    }

    #[test]
    fn cut_lines_bad_names_and_bytes_that_are_not_utf8_are_survived() {
        let mut text = b"Start-Date: 2026-11-05  07:10:00\nCommandline: apt upgrade \xff\xfe\nRequested-By: robin (1000)\n".to_vec();
        text.extend_from_slice(b"Upgrade: gnome-shell:amd64 (50.1-0ubuntu1.4, 50.1-0ubuntu1.5), Bad\xffName:amd64 (1, 2), gnome-shell-common:amd64 (50.1-0ub");
        let runs = parse(&text, &utc());
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].end, None, "the log ends inside the run");
        assert!(runs[0].commandline.as_deref().unwrap().starts_with("apt upgrade "));
        assert_eq!(runs[0].upgraded, [("gnome-shell".to_owned(), "50.1-0ubuntu1.5".to_owned())]);
        // lines before any Start-Date, a run with a broken date, noise
        let runs = parse(b"Upgrade: x1:amd64 (1, 2)\n\nStart-Date: soon\nUpgrade: y1:amd64 (1, 2)\nStart-Date: 2026-11-05  07:10:00\nnoise\n", &utc());
        assert_eq!(runs.len(), 1);
        assert!(runs[0].upgraded.is_empty());
    }

    #[test]
    fn gzip_rotation_a_cut_gzip_two_members_and_the_ends_of_big_files_are_read() {
        let dir = fixtures().join("var/log/apt");
        let plain = |name: &str| parse(&gunzip_tail(&fs::read(dir.join(name)).unwrap(), MAX_TEXT), &utc());
        let one = plain("history.log.1.gz");
        assert_eq!(one.len(), 2);
        assert!(one.iter().all(|r| r.end.is_some()));
        // .3.gz is cut in the middle of its third run: the first two come out whole, the third without End-Date
        let three = plain("history.log.3.gz");
        assert_eq!(three.len(), 3, "{three:?}");
        assert!(three[0].end.is_some() && three[1].end.is_some() && three[2].end.is_none());
        assert!(three[2].upgraded.len() >= 2, "the complete items before the cut");
        // .4.gz holds two gzip members (cat a.gz b.gz)
        assert_eq!(plain("history.log.4.gz").len(), 2);
        assert_eq!(gunzip_tail(b"not gzip at all", MAX_TEXT), b"");
        // review m4: beyond the bound the newest part is kept, from the first run that starts in it
        let raw = fs::read(dir.join("history.log.1.gz")).unwrap();
        let text = gunzip_tail(&raw, MAX_TEXT);
        let second = text.windows(12).position(|w| w == b"\nStart-Date:").unwrap() + 1;
        assert_eq!(gunzip_tail(&raw, text.len() - second), &text[second..]);
        assert_eq!(gunzip_tail(&raw, text.len() - second - 1), b"", "the second run cut too: nothing whole is left");
        assert_eq!(gunzip_tail(&raw, text.len()), text);
        let f = Fixture::new("history-tail");
        let file = f.base.join("history.log");
        let log = fs::read(dir.join("history.log")).unwrap();
        fs::write(&file, &log).unwrap();
        let last = log.windows(12).rposition(|w| w == b"\nStart-Date:").unwrap() + 1;
        assert_eq!(read_tail(&file, log.len() as u64).unwrap(), log);
        assert_eq!(read_tail(&file, log.len() as u64 + 100).unwrap(), log);
        assert_eq!(read_tail(&file, (log.len() - last) as u64).unwrap(), &log[last..]);
        let tail = parse(&read_tail(&file, (log.len() - last + 40) as u64).unwrap(), &utc());
        assert_eq!(tail.len(), 1, "the newest run, not the oldest part");
        assert_eq!(tail[0].start, Some(at("2026-11-05 07:10:00")));
    }

    #[test]
    fn groups_sources_and_outcomes_follow_the_fixture() {
        let (apt, releases) = fixture_logs();
        // our job: the PackageKit run of 4 Nov 12:00:01-12:03:40 lies in 12:00:00-12:03:42
        let windows = [(at("2026-11-04 12:00:00"), at("2026-11-04 12:03:42") + 500)];
        let all = entries(&apt, &releases, &context(&windows, &never));
        let find = |name: &str| all.iter().find(|e| e.name == name).unwrap_or_else(|| panic!("{name}: {:?}", names(&all)));
        assert_eq!((find("openssl").group, find("openssl").source, find("openssl").ok), ("updates", Some("ubuntu"), true));
        assert_eq!((find("libwebkit2gtk-4.1-0").group, find("libwebkit2gtk-4.1-0").source), ("updates", Some("us")));
        assert_eq!(find("libwebkit2gtk-4.1-0").version, "2.52.7-0ubuntu0.26.04.1");
        assert_eq!(find("fwupd").source, Some("app"), "PackageKit outside our windows");
        assert_eq!((find("gnome-boxes").group, find("gnome-boxes").source), ("software", Some("app")));
        assert!(!all.iter().any(|e| e.name == "libvirt-glib-1.0-0"), "an automatic dependency is not listed");
        assert!(!all.iter().any(|e| e.name == "rhythmbox"), "a removal is not listed");
        assert_eq!((find("vim").group, find("vim").source), ("software", Some("terminal")));
        assert_eq!((find("vim-common").group, find("vim-common").source), ("updates", Some("terminal")));
        assert_eq!((find("gimp").group, find("gimp").source), ("software", Some("terminal")), "aptitude");
        assert_eq!(find("ubuntu-drivers-common").source, Some("app"), "aptdaemon (Software Updater)");
        // a failed dpkg run (Error:), and the newest run, cut without End-Date while apt is not running
        assert!(!find("libsoup-3.0-0").ok);
        assert!(!find("gnome-shell").ok);
        assert!(!all.iter().any(|e| e.name == "gnome-shell-common"), "cut before its version");
        // review m2: the current dist-upgrade log is Software Updater's partial upgrade of 10 Oct (no release change):
        // its packages are updates of an app, not a release upgrade
        assert_eq!((find("libgtk-3-0t64").group, find("libgtk-3-0t64").source, find("libgtk-3-0t64").ok), ("updates", Some("app"), true));
        assert_eq!(find("ubuntu-release-upgrader-core").version, "1:26.04.15");
        assert_eq!(find("libgtk-3-0t64").date, at("2026-10-10 18:33:40"));
        // the release upgrades (the backups): 20 Aug (ok, no job window: no source) and an earlier failed attempt
        let ubuntu: Vec<&Entry> = all.iter().filter(|e| e.group == "releases").collect();
        assert_eq!(ubuntu.len(), 2, "{ubuntu:?}");
        assert_eq!((ubuntu[0].name.as_str(), ubuntu[0].version.as_str(), ubuntu[0].ok, ubuntu[0].source), ("Ubuntu", "26.04", true, None));
        assert_eq!(ubuntu[0].date, at("2026-08-20 10:42:01"));
        assert_eq!((ubuntu[1].version.as_str(), ubuntu[1].ok), ("26.04", false));
        assert!(!all.iter().any(|e| e.name == "base-files"), "a release upgrade is one entry");
        // newest first, the entries of one run in log order, every date its run's End-Date, every cursor unique
        assert!(all.windows(2).all(|w| w[0].cursor > w[1].cursor && w[0].date >= w[1].date), "ordered: {:?}", names(&all));
        assert!(all.iter().all(|e| e.cursor >= CURSOR_MIN && e.cursor < 1 << 53 && e.date % 1000 == 0));
        let vim = all.iter().position(|e| e.name == "vim-common").unwrap();
        assert_eq!(all[vim + 1].name, "vim", "Upgrade before Install inside one run");
        assert_eq!(find("vim-common").date, at("2026-11-03 19:20:30"));
        assert_eq!(find("vim-common").cursor >> SLOT_BITS, at("2026-11-03 19:20:30") / 1000);
        // while apt holds its lock, the newest run without End-Date is still running: not listed
        let busy = entries(&apt, &releases, &context(&windows, &always));
        assert!(!busy.iter().any(|e| e.name == "gnome-shell"));
        assert_eq!(busy.len(), all.len() - 1);
        // while the upgrader runs, the current dist-upgrade log is still being written: the partial upgrade waits
        let upgrading = Context { upgrading: &always, ..context(&windows, &never) };
        let during = entries(&apt, &releases, &upgrading);
        assert!(!during.iter().any(|e| e.name == "libgtk-3-0t64"));
        assert_eq!(during.iter().filter(|e| e.group == "releases").count(), 2);
        // another uid never counts as ours; a release upgrade inside our window is ours
        let other = Context { uid: 1001, ..context(&windows, &never) };
        assert_eq!(entries(&apt, &releases, &other).iter().find(|e| e.name == "libwebkit2gtk-4.1-0").unwrap().source, Some("app"));
        let release = [(at("2026-08-20 10:14:00"), at("2026-08-20 10:43:00"))];
        assert_eq!(entries(&apt, &releases, &context(&release, &never)).iter().find(|e| e.group == "releases").unwrap().source, Some("us"));
        assert_eq!(release_of("1:26.10.5"), "26.10");
        assert_eq!(release_of("26.04.14"), "26.04");
        assert_eq!(release_of("0.1"), "");
        assert!(backup_dir_name("20260820-1012") && !backup_dir_name("2026082-01012") && !backup_dir_name("history.log"));
    }

    #[test]
    fn a_current_release_upgrade_waits_for_the_upgrader_and_a_partial_upgrade_is_listed_per_package() {
        let log = |text: &str| Arc::new(parse(text.as_bytes(), &utc()));
        let upgrade = "Start-Date: 2026-11-05  09:00:00\nRequested-By: robin (1000)\n\
            Upgrade: ubuntu-release-upgrader-core:all (1:26.04.15, 1:26.10.3), gnome-shell:amd64 (50.1-0ubuntu1.5, 51.0-1ubuntu1)\n\
            End-Date: 2026-11-05  09:40:00\n";
        let partial = "Start-Date: 2026-11-04  09:00:00\nRequested-By: robin (1000)\n\
            Upgrade: update-manager-core:all (1:26.04.4, 1:26.04.5)\nEnd-Date: 2026-11-04  09:01:00\n";
        let releases = [Release { current: false, runs: log(partial) }, Release { current: true, runs: log(upgrade) }];
        let all = entries(&[], &releases, &context(&[], &never));
        assert_eq!(names(&all), ["releases Ubuntu 26.10", "updates update-manager-core 1:26.04.5"]);
        assert_eq!((all[0].date, all[1].source), (at("2026-11-05 09:40:00"), Some("app")));
        let upgrading = Context { upgrading: &always, ..context(&[], &never) };
        assert_eq!(names(&entries(&[], &releases, &upgrading)), ["updates update-manager-core 1:26.04.5"]);
        // a partial upgrade that apt is still writing (no End-Date, apt's lock held) waits like apt's own runs
        let running = [Release { current: true, runs: log("Start-Date: 2026-11-05  10:00:00\nUpgrade: libgtk-3-0t64:amd64 (1, 2)\n") }];
        assert!(entries(&[], &running, &context(&[], &always)).is_empty());
        assert_eq!(entries(&[], &running, &context(&[], &never))[0].ok, false, "a run that never ended failed");
    }

    #[test]
    fn pages_hold_200_entries_or_90_days_and_continue_exactly() {
        let (apt, releases) = fixture_logs();
        let all = entries(&apt, &releases, &context(&[], &never));
        let newest = all[0].date;
        let (first, more) = page(all.clone(), None, MAX_ENTRIES);
        assert!(more, "older than 90 days remain");
        assert!(first.iter().all(|e| e.date + WINDOW_MS >= newest));
        assert!(first.iter().any(|e| e.group == "releases"), "the release upgrade of 20 Aug is inside the window");
        assert!(first.iter().any(|e| e.name == "tzdata"), "{:?}", names(&first));
        let (second, more) = page(all.clone(), Some(first.last().unwrap().cursor), MAX_ENTRIES);
        assert!(!more);
        assert_eq!(first.len() + second.len(), all.len(), "nothing lost, nothing twice");
        assert!(second.iter().any(|e| e.name == "libexpat1"), "{:?}", names(&second));
        assert!(second.iter().any(|e| e.name == "libtiff6") && second.iter().any(|e| e.name == "htop"));
        // a limit that cuts inside one run continues with that run's next entry
        assert_eq!(walk(&all, 3), all);
        assert_eq!(page(Vec::new(), None, 10), (Vec::new(), false));
        assert_eq!(page(all.clone(), Some(CURSOR_MIN), 10), (Vec::new(), false));
        // the design's `before` in ms: the entries older than that time
        let (older, _) = page(all.clone(), Some(at("2026-11-03 19:20:30")), MAX_ENTRIES);
        assert_eq!(older[0].name, "gnome-boxes", "{:?}", names(&older));
        assert_eq!(page(all.clone(), Some(0), 10), (Vec::new(), false));
        // review m1: a run of 1200 packages and two runs that end in the same second, at limit 200 and at limit 1
        let t = at("2026-11-05 08:00:00");
        let apt = [
            run_of("big", 1200, t - 600_000, Some(t), "apt full-upgrade"),
            run_of("a", 2, t + 1000, Some(t + 5000), "unattended-upgrade"),
            run_of("b", 2, t + 3000, Some(t + 5000), "unattended-upgrade"),
        ];
        let many = entries(&apt, &[], &context(&[], &never));
        assert_eq!(many.len(), 1204);
        assert_eq!(names(&many[..4]), ["updates b0 1.0-1ubuntu0.1", "updates b1 1.0-1ubuntu0.1", "updates a0 1.0-1ubuntu0.1", "updates a1 1.0-1ubuntu0.1"]);
        assert_eq!(walk(&many, 200), many);
        assert_eq!(walk(&many, 1), many);
        assert_eq!(page(many.clone(), None, 200).0.len(), 200);
    }

    #[test]
    fn a_stale_job_file_ends_its_window_at_its_last_write_and_old_runs_of_ours_have_no_source() {
        let f = Fixture::new("history-job");
        let shared = shared_in(&f, "settings", false, true);
        let (store, paths) = (&shared.store, &shared.setup.paths);
        let boot = store::boot_id(paths).unwrap();
        let file = shared.ctx.cache_dir.join("update").join(store::JOB);
        let touch = |ms: u64| fs::File::options().write(true).open(&file).unwrap().set_modified(UNIX_EPOCH + Duration::from_millis(ms)).unwrap();
        let now = store::now_ms();
        let started = now - 86_400_000;
        let last = started + 60_000;
        // this boot, a live owner (this process): the job still runs, until now
        store.write(store::JOB, &json!({"jobId":"b","bootId":boot,"owner":store::current_owner(),"startedAt":started})).unwrap();
        touch(last);
        assert_eq!(job_windows(store, paths, now), [(started, now)]);
        // review M1: an earlier boot (a reboot during the download), an owner that is gone, no owner at all: until
        // the file's last write, never until now
        for job in [
            json!({"jobId":"b","bootId":"another-boot","owner":store::current_owner(),"startedAt":started}),
            json!({"jobId":"b","bootId":boot,"owner":{"pid":std::process::id(),"start":"0"},"startedAt":started}),
            json!({"jobId":"b","startedAt":started}),
        ] {
            store.write(store::JOB, &job).unwrap();
            touch(last);
            assert_eq!(job_windows(store, paths, now), [(started, last)], "{job}");
        }
        // finished: its own end; without startedAt: none
        store.write(store::JOB, &json!({"jobId":"b","startedAt":started,"finishedAt":started + 5000})).unwrap();
        assert_eq!(job_windows(store, paths, now), [(started, started + 5000)]);
        store.write(store::JOB, &json!({"jobId":"b"})).unwrap();
        assert!(job_windows(store, paths, now).is_empty());
        // GNOME Software's install a day after the stale job is an app's, not ours
        store.write(store::JOB, &json!({"jobId":"b","bootId":"another-boot","startedAt":started})).unwrap();
        touch(last);
        let windows = job_windows(store, paths, now);
        let software = run_of("gnome-maps", 1, started + 3_600_000, Some(started + 3_630_000), "packagekit role='install-packages'");
        let during = run_of("libwebkit", 1, started + 1000, Some(last), "packagekit role='update-packages'");
        let sources: Vec<_> = entries(&[during.clone(), software.clone()], &[], &context(&windows, &never)).iter().map(|e| e.source).collect();
        assert_eq!(sources, [Some("app"), Some("us")]);
        // review m3: a PackageKit run of ours older than jobs.jsonl's horizon has no window any more: no source
        let horizon = Context { horizon: started + 3_640_000, ..context(&[], &never) };
        let old: Vec<_> = entries(&[during.clone(), software], &[], &horizon).iter().map(|e| e.source).collect();
        assert_eq!(old, [None, None]);
        let kept = Context { horizon: started + 3_640_000, ..context(&windows, &never) };
        assert_eq!(entries(&[during], &[], &kept)[0].source, Some("us"), "a window that is still there counts");
        let apt = run_of("vim", 1, started, Some(started + 1000), "apt install vim");
        assert_eq!(entries(&[apt], &[], &horizon)[0].source, Some("terminal"), "only PackageKit runs of ours");
        assert_eq!(jobs_horizon(store, now), now - JOBS_KEPT_MS, "no jobs.jsonl: 90 days before now");
        store.append_job(&json!({"jobId":"a","startedAt":started,"finishedAt":started + 5000}), now).unwrap();
        let jobs = shared.ctx.cache_dir.join("update").join(store::JOBS);
        fs::File::options().write(true).open(&jobs).unwrap().set_modified(UNIX_EPOCH + Duration::from_millis(now - 10 * 86_400_000)).unwrap();
        assert_eq!(jobs_horizon(store, now), now - 10 * 86_400_000 - JOBS_KEPT_MS, "90 days before the last append");
    }

    #[test]
    fn update_history_reads_the_fixture_root_and_our_job_windows() {
        let f = Fixture::new("history");
        copy_tree(&fixtures().join("var"), &f.root.join("var"));
        let shared = shared_in(&f, "panel", false, true);
        let tz = glib::TimeZone::local();
        let (from, to) = (local_ms("2026-11-04 12:00:00", &tz).unwrap(), local_ms("2026-11-04 12:03:42", &tz).unwrap());
        shared.store.append_job(&json!({"jobId":"a","role":"update","startedAt":from,"finishedAt":to,"outcome":{"exit":1}}), to).unwrap();
        let result = history(&shared, &json!({}), Lang::Nl).unwrap();
        let entries = result["entries"].as_array().unwrap();
        assert_eq!(result["more"], true);
        let webkit = entries.iter().find(|e| e["name"] == "libwebkit2gtk-4.1-0").unwrap();
        let uid = unsafe { libc::getuid() };
        assert_eq!(webkit["source"], if uid == 1000 { "us" } else { "app" }, "the fixture's Requested-By is uid 1000");
        assert_eq!((webkit["group"].as_str(), webkit["version"].as_str(), webkit["ok"].as_bool()), (Some("updates"), Some("2.52.7-0ubuntu0.26.04.1"), Some(true)));
        assert_eq!(webkit["date"].as_u64(), local_ms("2026-11-04 12:03:40", &tz));
        let keys: Vec<&String> = webkit.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["group", "name", "version", "date", "ok", "source", "cursor"]);
        let last = entries.last().unwrap()["cursor"].as_u64().unwrap();
        let next = history(&shared, &json!({"before":last,"limit":200}), Lang::Nl).unwrap();
        assert_eq!(next["more"], false);
        assert!(!next["entries"].as_array().unwrap().is_empty());
        assert!(next["entries"].as_array().unwrap().iter().all(|e| e["cursor"].as_u64() < Some(last)));
        assert_eq!(history(&shared, &json!({"limit":1}), Lang::En).unwrap()["entries"].as_array().unwrap().len(), 1);
        // the finished jobs of jobs.jsonl
        assert_eq!(job_windows(&shared.store, &shared.setup.paths, to + 60_000), [(from, to)]);
        // no program ran
        assert_eq!(shared.setup.tools.runs(), 0);
        assert_eq!(f.stub_log(), "");
        // an empty fixture root (dry-run, a settings host): no history, nothing more
        let empty = Fixture::new("history-empty");
        write(&empty.root.join("var/log/apt/history.log"), "");
        let shared = shared_in(&empty, "settings", true, true);
        assert_eq!(history(&shared, &json!({}), Lang::En).unwrap(), json!({"entries":[],"more":false}));
    }
}
