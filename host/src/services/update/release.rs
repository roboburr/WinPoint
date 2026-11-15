//! Release detection and the LTS/normal choice (design §6.1, §6.3, §4.12 C14), GW5.
//!
//! * **Prompt** ([`read_prompt`]): `/etc/update-manager/release-upgrades`, then `release-upgrades.d/*.cfg` in byte
//!   order; the last file with a `Prompt` key in its `[DEFAULT]` section decides, each file read as Python's
//!   configparser reads it ([`config_prompt`]: keys are case-insensitive, `=` or `:`, the whole rest of the line,
//!   indented continuation lines included, is the value; a file configparser refuses, e.g. with a key twice, is
//!   ignored as Ubuntu ignores it). `never`/`no` → never, `lts` → lts,
//!   anything else (also no key) → normal. `lts` on a release that is not an LTS behaves as normal (Ubuntu's rule,
//!   the file's own comment); the page shows `promptLtsInterim` then (`currentLts`). `promptFile` names the deciding
//!   file as its system path; `override` is that file when it is a `.d` file (the page disables the control).
//! * **Offer** ([`detect`]): the meta-release copy Ubuntu itself reads for this Prompt (`meta-release-lts` for lts
//!   on an LTS release, else `meta-release`), in the user's cache (`$XDG_CACHE_HOME/update-manager-core/`) and in
//!   root's `/var/lib/update-manager/`: the newer file that parses wins. Parsed as `MetaRelease.parse` does: every
//!   stanza needs Dist, Version, Supported and Date, or the whole file is refused; the current release (os-release
//!   `VERSION_CODENAME`) must be listed; the first later-dated stanza with `Supported: 1` is the offer (Version,
//!   Name, ReleaseNotesHtml, UpgradeBroken). Without an offer in that copy, the motd stamp
//!   `/var/lib/ubuntu-release-upgrader/release-upgrade-available` counts when it is newer than the copy and names a
//!   later release (only the version is read, `\d\d.\d\d(.\d)?( LTS)?`, because the text is in the timer's
//!   locale; only year.month is compared, so a stale "26.04.1 LTS" on 26.04 is not an offer); its codename then
//!   comes from distro-info and is never the current one. Prompt=never offers nothing.
//! * **distro-info** (`/usr/share/distro-info/ubuntu.csv`, columns by header name): the offered release's `eol`
//!   and `supportMonths` (release date → eol), and `released`: the newest release that is out (release ≤ today),
//!   still supported (eol > today) and later than the current one; for lts on an LTS release only LTS versions.
//!   The page shows `relNotOffered` while it is not offered (today: 26.10 is out, nothing is offered).
//! * **`update.release.check`** ([`check`]): a write method, so only the page's click starts it (status and every
//!   read run no program). At most once per 24 h (after a failed run once per hour), not while metered unless
//!   `update.metered`, not offline, not for Prompt=never, one at a time; otherwise `{started:false, skipped:<why>}`. On its own thread:
//!   `check-new-release -q` from the tool table (a stub in test mode, never the real one in a test) as the user,
//!   nice 10, `LANG=C`, 60 s; exit 0 = a new release, 1 = none, anything else `update.release.err.checkFailed`
//!   (`release.checkError`). `release.json` keeps `lastRun`, `rc` and the error; then `update.changed`. Never in
//!   dry-run (`update.rs` answers first).
//! * **`update.release.setPrompt`** ([`set_prompt`]): no password for nothing (the file already says it, or a `.d`
//!   file decides: `{prompt, override}` as it is); else `admin::run` (pkexec + the helper, or `/bin/sh -c` with the
//!   same script), the file read again (it must now say the value as Ubuntu reads it, else
//!   `update.release.err.promptFailed`: never a silent no-op), `update.changed`.
//! * **`update.release.ubuntuHold`** ([`ubuntu_hold`]): on (only for the offered codename while GW6's verdict is
//!   `hold`/`blocked`, `compat::cached`) writes `com.ubuntu.update-manager check-new-release-ignore = <codename>`
//!   and `check-dist-upgrades = false`, keeping the previous values in `settings.update.ubuntuHoldPrevious
//!   {ignore, checkDist, codename}` (null: the key had no user value); off restores exactly those, and only keys
//!   that still hold what we wrote ([`restore_hold`]; GW6 calls it too when the verdict becomes ok/warn or the
//!   codename changes). GSettings as in options.rs: the default source and backend (dconf) in production; in test
//!   mode only the fixture root's compiled schemas and a keyfile backend in it, never dconf.
use super::{Shared, Stats, admin, compat, net, paths::Paths, paths::Tool, store};
use crate::{
    files::{ApiError, ApiResult},
    i18n::{Lang, NO_PARAMS},
};
use gio::prelude::*;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Mutex, atomic::AtomicU64, atomic::Ordering},
    time::{Duration, UNIX_EPOCH},
};

/// The directory Ubuntu's release upgrader reads its Prompt from, as the page names it (`promptFile`, `override`).
const ETC_SHOWN: &str = "/etc/update-manager";
const MAIN_FILE: &str = "release-upgrades";
const PARTS_DIR: &str = "release-upgrades.d";
const STAMP: &str = "release-upgrade-available";
/// update-manager-core's own copies in the user's cache (`MetaRelease.py`: `$XDG_CACHE_HOME/update-manager-core`).
const USER_COPIES: &str = "update-manager-core";
pub(crate) const RELEASE: &str = "release.json";
const CHECK_EVERY_MS: u64 = 24 * 3600 * 1000;
/// After a check that failed (no answer from check-new-release), a click may try again sooner.
const CHECK_RETRY_MS: u64 = 3600 * 1000;
const CHECK_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_TEXT: u64 = 1 << 20;
pub(crate) const SCHEMA: &str = "com.ubuntu.update-manager";
pub(crate) const KEY_IGNORE: &str = "check-new-release-ignore";
pub(crate) const KEY_DIST: &str = "check-dist-upgrades";
const PREVIOUS: &str = "update.ubuntuHoldPrevious";

// ------------------------------------------------------------------------------------------------ Prompt

/// The effective `Prompt=` (module doc).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Prompt {
    /// "lts" | "normal" | "never"
    pub value: &'static str,
    /// the main file's own value (lowercased), None without a key there
    pub main: Option<String>,
    /// the deciding file as its system path (the main file when no file has the key)
    pub file: String,
    /// the deciding file when it is a `release-upgrades.d/*.cfg`
    pub overridden: Option<String>,
}
pub(crate) fn prompt_value(raw: Option<&str>) -> &'static str {
    match raw {
        Some("never" | "no") => "never",
        Some("lts") => "lts",
        _ => "normal",
    }
}
/// Python's whitespace (`str.isspace`, `re`'s `\s`): Rust's White_Space plus U+001C..U+001F.
fn py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}
/// The lines of a file as Python's universal newlines read them: `\r\n`, `\r` and `\n` each end a line.
fn py_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let bytes = text.as_bytes();
    let (mut start, mut i) = (0, 0);
    while i < bytes.len() {
        if bytes[i] == b'\n' || bytes[i] == b'\r' {
            lines.push(&text[start..i]);
            i += if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') { 2 } else { 1 };
            start = i;
        } else {
            i += 1;
        }
    }
    if start < bytes.len() {
        lines.push(&text[start..]);
    }
    lines
}
/// The `Prompt` of the `[DEFAULT]` section as Ubuntu reads it, lowercased; None without one, and None when Ubuntu
/// ignores the whole file. `MetaRelease._parse_cfg` reads each file with its own `configparser.ConfigParser()`
/// (Python 3.14's `_read_inner` with the defaults: strict, `#`/`;` full-line comments only, blank lines inside values)
/// and skips a file on any `configparser.Error`. So, line by line: a comment or blank line never ends a value; a line
/// indented deeper than the current key's line continues its value; a section header is `[` + name + the last `]`
/// (`[DEFAULT] ; x` is DEFAULT, `[default]` is not); a key ends at the first `=` or `:` and is case-insensitive; a key
/// before any section, a line that is neither, an empty key, a key twice in one section, or a section other than
/// DEFAULT twice make the file ignored. Fixtures: `tests/update/fixtures/release/cfg/expected.tsv`.
pub(crate) fn config_prompt(text: &str) -> Option<String> {
    let mut sections: BTreeSet<&str> = BTreeSet::new();
    let mut keys: BTreeSet<(&str, String)> = BTreeSet::new();
    let mut section: Option<&str> = None;
    let mut key: Option<String> = None;
    let mut level = 0usize;
    let mut prompt: Option<Vec<&str>> = None;
    let mut errors = false;
    for line in py_lines(text) {
        let trimmed = line.trim_matches(py_space);
        let comment = trimmed.starts_with(['#', ';']);
        let in_prompt = section == Some("DEFAULT") && key.as_deref() == Some("prompt");
        if trimmed.is_empty() || comment {
            if let Some(parts) = prompt.as_mut().filter(|_| !comment && in_prompt) {
                parts.push("");
            }
            continue;
        }
        let indent = line.chars().take_while(|c| py_space(*c)).count();
        if section.is_some() && key.is_some() && indent > level {
            if let Some(parts) = prompt.as_mut().filter(|_| in_prompt) {
                parts.push(trimmed);
            }
            continue;
        }
        level = indent;
        let close = trimmed.rfind(']').filter(|at| trimmed.starts_with('[') && *at >= 2);
        if let Some(close) = close {
            let name = &trimmed[1..close];
            if name != "DEFAULT" && !sections.insert(name) {
                return None; // DuplicateSectionError
            }
            section = Some(name);
            key = None;
            continue;
        }
        let Some(name) = section else { return None }; // MissingSectionHeaderError
        let Some(at) = trimmed.find(['=', ':']) else {
            errors = true; // ParsingError (raised at the end of the file)
            continue;
        };
        let option = trimmed[..at].trim_end_matches(py_space).to_lowercase();
        errors |= option.is_empty();
        if !keys.insert((name, option.clone())) {
            return None; // DuplicateOptionError
        }
        if name == "DEFAULT" && option == "prompt" {
            prompt = Some(vec![trimmed[at + 1..].trim_matches(py_space)]);
        }
        key = (!option.is_empty()).then_some(option);
    }
    if errors {
        return None;
    }
    prompt.map(|parts| parts.join("\n").trim_end_matches(py_space).to_lowercase())
}
/// `release-upgrades`, then `release-upgrades.d/*.cfg` sorted; None without the directory (no Ubuntu upgrader).
pub(crate) fn read_prompt(paths: &Paths) -> Option<Prompt> {
    let dir = &paths.update_manager_etc;
    if !dir.is_dir() {
        return None;
    }
    let main = text(&dir.join(MAIN_FILE)).and_then(|t| config_prompt(&t));
    let mut decided: Option<(String, Option<String>)> = main.clone().map(|v| (MAIN_FILE.to_owned(), Some(v)));
    let mut parts: Vec<String> = fs::read_dir(dir.join(PARTS_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".cfg"))
        .collect();
    parts.sort();
    for name in parts {
        if let Some(v) = text(&dir.join(PARTS_DIR).join(&name)).and_then(|t| config_prompt(&t)) {
            decided = Some((format!("{PARTS_DIR}/{name}"), Some(v)));
        }
    }
    let (rel, raw) = decided.unwrap_or_else(|| (MAIN_FILE.to_owned(), None));
    let file = format!("{ETC_SHOWN}/{rel}");
    Some(Prompt {
        value: prompt_value(raw.as_deref()),
        main,
        overridden: (rel != MAIN_FILE).then(|| file.clone()),
        file,
    })
}
fn text(path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    (meta.is_file() && meta.len() <= MAX_TEXT).then(|| fs::read_to_string(path).ok()).flatten()
}
fn mtime_ms(path: &Path) -> Option<u64> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    Some(modified.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64)
}

// ------------------------------------------------------------------------------------------------ os-release

/// This release: its codename, `VERSION_ID`, and whether it is an LTS.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Current {
    pub codename: Option<String>,
    pub version_id: Option<String>,
    pub lts: bool,
}
pub(crate) fn parse_os_release(text: &str) -> Current {
    let field = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
            .map(|v| v.trim().trim_matches('"').trim_matches('\'').to_owned())
            .filter(|v| !v.is_empty())
    };
    Current {
        codename: field("VERSION_CODENAME").or_else(|| field("UBUNTU_CODENAME")),
        version_id: field("VERSION_ID"),
        lts: field("VERSION").is_some_and(|v| v.contains("LTS")),
    }
}

// ------------------------------------------------------------------------------------------------ distro-info

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DistroRow {
    pub version: String,
    pub series: String,
    pub release: Option<String>,
    pub eol: Option<String>,
}
/// ubuntu.csv by header name (`version,codename,series,created,release,eol,…`; later columns may be missing).
pub(crate) fn parse_distro_info(text: &str) -> Vec<DistroRow> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let Some(header) = lines.next() else { return Vec::new() };
    let names: Vec<&str> = header.split(',').map(str::trim).collect();
    let at = |name: &str| names.iter().position(|n| *n == name);
    let (Some(version), Some(series), release, eol) = (at("version"), at("series"), at("release"), at("eol")) else {
        return Vec::new();
    };
    lines
        .map(|line| {
            let cells: Vec<&str> = line.split(',').map(str::trim).collect();
            let cell = |i: Option<usize>| i.and_then(|i| cells.get(i)).filter(|c| !c.is_empty()).map(|c| c.to_string());
            DistroRow {
                version: cell(Some(version)).unwrap_or_default(),
                series: cell(Some(series)).unwrap_or_default(),
                release: cell(release).filter(|d| ymd(d).is_some()),
                eol: cell(eol).filter(|d| ymd(d).is_some()),
            }
        })
        .filter(|r| !r.series.is_empty())
        .collect()
}
fn ymd(date: &str) -> Option<(i64, i64, i64)> {
    let mut parts = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    (parts.next().is_none() && (1..=12).contains(&m) && (1..=31).contains(&d)).then_some((y, m, d))
}
/// Whole months from `from` to `to` (2026-10-15 → 2027-07-15: 9).
pub(crate) fn months_between(from: &str, to: &str) -> Option<i64> {
    let ((y1, m1, d1), (y2, m2, d2)) = (ymd(from)?, ymd(to)?);
    let months = (y2 - y1) * 12 + (m2 - m1) - i64::from(d2 < d1);
    Some(months.max(0))
}
/// The numbers of a version ("26.04.1 LTS" → [26, 4, 1]), for "later than".
fn version_key(version: &str) -> Vec<u32> {
    version
        .split_whitespace()
        .next()
        .unwrap_or("")
        .split('.')
        .map_while(|p| p.parse::<u32>().ok())
        .collect()
}

// ------------------------------------------------------------------------------------------------ meta-release

/// One `Dist` stanza (module doc).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Dist {
    pub dist: String,
    pub name: Option<String>,
    pub version: String,
    pub date: [u32; 6],
    pub supported: bool,
    pub notes: Option<String>,
    pub broken: Option<String>,
}
/// apt's tag file as `MetaRelease.parse` reads it; None when a stanza lacks Dist/Version/Supported/Date or one of
/// them does not parse (Ubuntu then refuses the file too), and None without any stanza (MetaRelease deletes an empty
/// copy before it downloads, so an empty copy is no copy).
pub(crate) fn parse_meta_release(text: &str) -> Option<Vec<Dist>> {
    let mut dists = Vec::new();
    for stanza in stanzas(text) {
        let get = |key: &str| stanza.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.clone());
        let supported: i64 = get("Supported")?.trim().parse().ok()?;
        dists.push(Dist {
            dist: get("Dist")?,
            name: get("Name"),
            version: get("Version")?,
            date: parse_date(&get("Date")?)?,
            supported: supported != 0,
            notes: get("ReleaseNotesHtml"),
            broken: get("UpgradeBroken"),
        });
    }
    (!dists.is_empty()).then_some(dists)
}
fn stanzas(text: &str) -> Vec<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut cur: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else if line.starts_with([' ', '\t']) {
            if let Some((_, v)) = cur.last_mut() {
                v.push('\n');
                v.push_str(line.trim());
            }
        } else if !line.starts_with('#')
            && let Some((k, v)) = line.split_once(':')
        {
            cur.push((k.trim().to_owned(), v.trim().to_owned()));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}
/// An RFC 2822 date as `email.utils.parsedate` reads it ("Thu, 15 October 2026 12:00:00 UTC"; 3-letter or full
/// English month names; the zone is ignored, as Ubuntu ignores it) → [y, m, d, H, M, S].
pub(crate) fn parse_date(text: &str) -> Option<[u32; 6]> {
    const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    const LONG: [&str; 12] =
        ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];
    let month = |t: &str| {
        let t = t.to_ascii_lowercase();
        MONTHS.iter().chain(LONG.iter()).position(|m| *m == t).map(|i| (i % 12) as u32 + 1)
    };
    let tokens: Vec<&str> = text.split(|c: char| c.is_whitespace() || c == ',').filter(|t| !t.is_empty()).collect();
    let at = tokens.iter().position(|t| month(t).is_some())?;
    let mon = month(tokens[at])?;
    let num = |t: &str| t.parse::<u32>().ok();
    let (day, next) = match at.checked_sub(1).and_then(|i| num(tokens[i])) {
        Some(day) => (day, at + 1),
        None => (num(tokens.get(at + 1)?)?, at + 2),
    };
    let year = match num(tokens.get(next)?)? {
        y @ 0..=68 => y + 2000,
        y @ 69..=99 => y + 1900,
        y => y,
    };
    let mut hms = tokens.get(next + 1).map_or("0:0:0", |t| t).split(':').map(num);
    let (h, m) = (hms.next()??, hms.next().flatten().unwrap_or(0));
    let s = hms.next().flatten().unwrap_or(0);
    ((1..=31).contains(&day) && h < 24 && m < 60 && s < 61).then_some([year, mon, day, h, m, s])
}
/// The release `MetaRelease` offers from these stanzas (module doc): the current one must be listed.
pub(crate) fn offer<'a>(dists: &'a [Dist], current: &str) -> Option<&'a Dist> {
    let now = dists.iter().find(|d| d.dist == current)?;
    dists.iter().find(|d| d.date > now.date && d.supported)
}
/// The version in the motd stamp (`\d\d\.\d\d(\.\d)?( LTS)?`), whatever language the sentence around it is in.
pub(crate) fn stamp_version(text: &str) -> Option<String> {
    let b = text.as_bytes();
    let digit = |i: usize| b.get(i).is_some_and(u8::is_ascii_digit);
    for i in 0..b.len() {
        if i > 0 && (b[i - 1].is_ascii_digit() || b[i - 1] == b'.') {
            continue;
        }
        if !(digit(i) && digit(i + 1) && b.get(i + 2) == Some(&b'.') && digit(i + 3) && digit(i + 4)) || digit(i + 5) {
            continue;
        }
        let mut end = i + 5;
        if b.get(end) == Some(&b'.') && digit(end + 1) && !digit(end + 2) {
            end += 2;
        }
        if text[end..].starts_with(" LTS") {
            end += 4;
        }
        return Some(text[i..end].to_owned());
    }
    None
}

// ------------------------------------------------------------------------------------------------ detection

/// What Ubuntu offers this PC (module doc).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Offer {
    pub version: String,
    pub codename: String,
    pub name: Option<String>,
    pub notes: Option<String>,
    pub broken: Option<String>,
    pub eol: Option<String>,
    pub support_months: Option<i64>,
    /// "cache" (a meta-release copy) or "stamp" (the motd stamp)
    pub source: &'static str,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Detection {
    pub prompt: Option<Prompt>,
    pub current: Current,
    pub offer: Option<Offer>,
    /// (version, codename) of the newest release that is out (module doc)
    pub released: Option<(String, String)>,
    /// the modification time of the copy that decided, in ms
    pub checked_at: Option<u64>,
}
/// Today as `YYYY-MM-DD` (local time: distro-info's dates are calendar days).
pub(crate) fn today() -> String {
    glib::DateTime::now_local()
        .ok()
        .and_then(|d| d.format("%Y-%m-%d").ok())
        .map(|s| s.to_string())
        .unwrap_or_default()
}
/// The meta-release copies of this Prompt, user's first (`name` is `meta-release` or `meta-release-lts`).
fn copies(paths: &Paths, name: &str) -> [PathBuf; 2] {
    [paths.user_cache.join(USER_COPIES).join(name), paths.update_manager_var.join(name)]
}
pub(crate) fn detect(paths: &Paths, today: &str) -> Detection {
    let current = parse_os_release(&text(&paths.os_release).unwrap_or_default());
    let prompt = read_prompt(paths);
    let rows = parse_distro_info(&text(&paths.distro_info).unwrap_or_default());
    let row = |series: &str| rows.iter().find(|r| r.series == series);
    let effective = prompt.as_ref().map_or("normal", |p| p.value);
    let lts_only = effective == "lts" && current.lts;
    let mut detection = Detection { current: current.clone(), ..Detection::default() };
    // released (distro-info): out, supported, after this release
    if let Some(now) = current.codename.as_deref().and_then(row).and_then(|r| r.release.clone()) {
        detection.released = rows
            .iter()
            .filter(|r| Some(r.series.as_str()) != current.codename.as_deref())
            .filter(|r| r.release.as_deref().is_some_and(|d| d > now.as_str() && d <= today))
            .filter(|r| r.eol.as_deref().is_none_or(|e| e > today))
            .filter(|r| !lts_only || r.version.contains("LTS"))
            .max_by(|a, b| a.release.cmp(&b.release))
            .map(|r| (r.version.clone(), r.series.clone()));
    }
    if effective != "never"
        && let Some(codename) = current.codename.clone()
    {
        let name = if lts_only { "meta-release-lts" } else { "meta-release" };
        // the newer copy that parses
        let mut found: Vec<(u64, Vec<Dist>)> = copies(paths, name)
            .iter()
            .filter_map(|p| Some((mtime_ms(p)?, parse_meta_release(&text(p)?)?)))
            .collect();
        found.sort_by(|a, b| b.0.cmp(&a.0));
        let copy = found.into_iter().next();
        detection.checked_at = copy.as_ref().map(|(at, _)| *at);
        let cached = copy.as_ref().and_then(|(_, dists)| offer(dists, &codename)).map(|d| Offer {
            version: d.version.clone(),
            codename: d.dist.clone(),
            name: d.name.clone(),
            notes: d.notes.clone(),
            broken: d.broken.clone(),
            eol: None,
            support_months: None,
            source: "cache",
        });
        // the motd stamp: only newer than the copy, only a later version, its codename from distro-info
        let stamp_path = paths.release_upgrader_var.join(STAMP);
        let stamped = || {
            let at = mtime_ms(&stamp_path)?;
            if detection.checked_at.is_some_and(|copy| copy > at) {
                return None;
            }
            let version = stamp_version(&text(&stamp_path)?)?;
            // the release (year.month) only: a stale "26.04.1 LTS" stamp is not later than 26.04
            let release = |v: &str| version_key(v).into_iter().take(2).collect::<Vec<_>>();
            let short = release(&version);
            if short <= release(current.version_id.as_deref()?) || (lts_only && !version.contains("LTS")) {
                return None;
            }
            let found = rows.iter().find(|r| release(&r.version) == short)?;
            if Some(found.series.as_str()) == current.codename.as_deref() {
                return None;
            }
            Some(Offer {
                version: version.clone(),
                codename: found.series.clone(),
                name: None,
                notes: None,
                broken: None,
                eol: None,
                support_months: None,
                source: "stamp",
            })
        };
        detection.offer = cached.or_else(stamped).map(|mut o| {
            if let Some(r) = row(&o.codename) {
                o.eol = r.eol.clone();
                o.support_months = r.release.as_deref().zip(r.eol.as_deref()).and_then(|(a, b)| months_between(a, b));
            }
            o
        });
    }
    detection.prompt = prompt;
    detection
}
/// Unit tests read release files only below a fixture root (never this machine's `/var` or cache).
fn reads_files(paths: &Paths) -> bool {
    !cfg!(test) || paths.fixture
}

/// The snapshot's `release` object (design §4.3; GW6 adds the card's `state`, `gnomeMajor`, `reasons`, GW7
/// `startRc`). `compose` sets `state:'running'` while Ubuntu's upgrader runs.
pub(super) fn status(shared: &Shared) -> Value {
    let paths = &shared.setup.paths;
    let d = if reads_files(paths) { detect(paths, &today()) } else { Detection::default() };
    let record = shared.store.read(RELEASE).unwrap_or(Value::Null);
    let lang = shared.ctx.lang();
    let check_error = record["error"]["key"].as_str().filter(|k| k.starts_with("update.release.")).map(|key| {
        json!({"code":record["error"]["code"],"message":crate::i18n::tr(lang, key),"key":key,"params":{}})
    });
    let o = d.offer.as_ref();
    // GW6: the object is bound so the compatibility verdict can be added (only `let mut view =` and the tail changed)
    let mut view = json!({"state":"none","ubuntuPromptHeld":held(shared),"startRc":null,"offered":o.is_some(),
           "released":d.released.as_ref().map(|(v, c)| json!({"version":v,"codename":c})),
           "version":o.map(|o| &o.version),"codename":o.map(|o| &o.codename),"name":o.and_then(|o| o.name.as_ref()),
           "eol":o.and_then(|o| o.eol.as_ref()),"supportMonths":o.and_then(|o| o.support_months),
           "notesUrl":o.and_then(|o| o.notes.as_ref()),"upgradeBroken":o.and_then(|o| o.broken.as_ref()),
           "source":o.map(|o| o.source),"gnomeMajor":null,"reasons":[],
           "prompt":d.prompt.as_ref().map(|p| p.value),"promptFile":d.prompt.as_ref().map(|p| &p.file),
           "override":d.prompt.as_ref().and_then(|p| p.overridden.as_ref()),"currentLts":d.current.lts,
           "helper":admin::helpers(&shared.setup),"checkedAt":d.checked_at,
           "lastCheck":record["lastRun"],"checkError":check_error});
    // GW6: the card's verdict (`state`, `verdict`, `gnomeMajor`, `reasons`, `allowAnyway`, `compat`, `upgraderNl`;
    // compat.rs, design §6.2) for the offered release; GW7 may then turn `state` into its own states. Ubuntu's own
    // prompt (§4.12, C14) goes back when the verdict now allows the upgrade, or another release is offered than the
    // one we held it for: only in a Settings host, never in a dry run (restore_hold touches only keys that still
    // hold our values).
    compat::fill(shared, &mut view, o);
    let held_for = shared.ctx.settings()["update"]["ubuntuHoldPrevious"]["codename"].as_str().map(str::to_owned);
    if let Some(codename) = held_for
        && shared.ctx.kind == "settings"
        && !(shared.ctx.dry_run || crate::backend::dry_run())
    {
        let allows = matches!(view["verdict"].as_str(), Some("ok" | "warn")) && view["codename"] == codename.as_str();
        let moved = o.is_some_and(|o| o.codename != codename);
        if (allows || moved) && restore_hold(shared).is_ok() {
            view["ubuntuPromptHeld"] = json!(held(shared));
        }
    }
    view
    // GW6 end
}

/// `update.changed` after a release change (a check, a Prompt change): the page re-reads `update.status` when
/// `rev` moved. The snapshot's own `rev` belongs to the pk thread, so this one is the next value without writing
/// it (the page's `rev` is always the snapshot's, so it reads again; design §4.4).
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

// ------------------------------------------------------------------------------------------------ check

/// The services (by cache directory) whose check-new-release runs now: one at a time.
static RUNNING: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

/// Why a click does not run check-new-release now (None: it runs).
fn skip(shared: &Shared, now: u64) -> Option<&'static str> {
    let paths = &shared.setup.paths;
    if read_prompt(paths).is_some_and(|p| p.value == "never") {
        return Some("never");
    }
    let snapshot = shared.store.read(store::SNAPSHOT).unwrap_or(Value::Null);
    let network = net::Net::from_json(&snapshot["network"]);
    if network.refuses_check() {
        return Some("offline");
    }
    if network.metered && shared.ctx.settings()["update"]["metered"] != true {
        return Some("metered");
    }
    let record = shared.store.read(RELEASE).unwrap_or(Value::Null);
    let every = if record["error"].is_null() { CHECK_EVERY_MS } else { CHECK_RETRY_MS };
    if record["lastRun"].as_u64().is_some_and(|at| now.saturating_sub(at) < every) {
        return Some("recent");
    }
    None
}
/// `update.release.check {}` → `{started:true}`, or `{started:false, skipped}` (module doc).
pub(super) fn check(shared: &Shared, _lang: Lang) -> ApiResult {
    let now = store::now_ms();
    if let Some(reason) = skip(shared, now) {
        return Ok(json!({"started":false,"skipped":reason}));
    }
    let key = shared.ctx.cache_dir.clone();
    if !RUNNING.lock().unwrap_or_else(|e| e.into_inner()).insert(key.clone()) {
        return Ok(json!({"started":false,"skipped":"running"}));
    }
    // the thread's own view of the service (the pk thread keeps the original)
    let copy = Shared {
        ctx: shared.ctx.clone(),
        setup: shared.setup.clone(),
        store: shared.store.clone(),
        stats: Stats::default(),
        rev: AtomicU64::new(shared.rev.load(Ordering::SeqCst)),
    };
    shared.stats.threads.fetch_add(1, Ordering::SeqCst);
    let spawned = std::thread::Builder::new().name("update-release".into()).spawn(move || {
        run_check(&copy, now);
        announce(&copy);
        RUNNING.lock().unwrap_or_else(|e| e.into_inner()).remove(&copy.ctx.cache_dir);
    });
    if spawned.is_err() {
        RUNNING.lock().unwrap_or_else(|e| e.into_inner()).remove(&key);
        return Err(ApiError::tr("failed", shared.ctx.lang(), "update.release.err.checkFailed", NO_PARAMS));
    }
    Ok(json!({"started":true}))
}
/// One `check-new-release -q` (module doc); writes `release.json` and returns its record.
pub(crate) fn run_check(shared: &Shared, now: u64) -> Value {
    let setup = &shared.setup;
    let rc = setup.tools.command(Tool::CheckNewRelease).ok().and_then(|mut command| {
        command.arg("-q").env("LANG", "C").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        // update-manager-core writes its copy to $XDG_CACHE_HOME/update-manager-core (read again below)
        command.env("XDG_CACHE_HOME", &setup.paths.user_cache).env("HOME", &setup.paths.user_home);
        if !setup.test {
            for key in ["http_proxy", "https_proxy", "no_proxy", "HTTP_PROXY", "HTTPS_PROXY", "NO_PROXY"] {
                if let Some(value) = std::env::var_os(key) {
                    command.env(key, value);
                }
            }
        }
        // nice 10, in the child before it starts (no extra program)
        unsafe {
            command.pre_exec(|| {
                libc::nice(10);
                Ok(())
            });
        }
        admin::wait(command.spawn().ok()?, CHECK_TIMEOUT)?.rc
    });
    let error = match rc {
        Some(0 | 1) => Value::Null,
        _ => json!({"code":"check_failed","key":"update.release.err.checkFailed"}),
    };
    let mut record = shared.store.read(RELEASE).filter(Value::is_object).unwrap_or_else(|| json!({}));
    record["lastRun"] = json!(now);
    record["rc"] = json!(rc);
    record["error"] = error;
    record["newRelease"] = json!(rc == Some(0));
    let _ = shared.store.write(RELEASE, &record);
    record
}

// ------------------------------------------------------------------------------------------------ setPrompt

fn prompt_failed(lang: Lang) -> ApiError {
    ApiError::tr("failed", lang, "update.release.err.promptFailed", NO_PARAMS)
}
/// `update.release.setPrompt {prompt:'lts'|'normal'|'never'}` → `{prompt, override, changed}` (module doc).
pub(super) fn set_prompt(shared: &Shared, args: &Value, lang: Lang) -> ApiResult {
    let want = match args["prompt"].as_str() {
        Some("lts") => "lts",
        Some("normal") => "normal",
        Some("never") => "never",
        _ => return Err(ApiError::tr("bad_args", lang, "update.err.badArgs", NO_PARAMS)),
    };
    let paths = &shared.setup.paths;
    let Some(before) = read_prompt(paths) else { return Err(prompt_failed(lang)) };
    let answer = |p: &Prompt, changed: bool| json!({"prompt":p.value,"override":p.overridden,"changed":changed});
    // no password for nothing: a .d file decides, or the main file already says it (no key there is normal)
    if before.overridden.is_some() || (before.value == want && (before.main.is_some() || want == "normal")) {
        return Ok(answer(&before, false));
    }
    admin::run(shared, Tool::PromptHelper, &["set-prompt", want], lang)?;
    let after = read_prompt(paths);
    announce(shared);
    // the helper's @@ok is not enough: the file must now say it the way Ubuntu reads it (a .d file that appeared
    // meanwhile decides instead, and the answer names it)
    match after {
        Some(after) if after.overridden.is_some() || after.main.as_deref() == Some(want) => Ok(answer(&after, true)),
        _ => Err(prompt_failed(lang)),
    }
}

// ------------------------------------------------------------------------------------------------ ubuntuHold

/// update-manager's settings object, or None (no schema or keys; test mode without a fixture root).
pub(crate) fn update_manager(setup: &super::paths::Setup) -> Option<gio::Settings> {
    let (schema, backend) = if setup.test {
        let dir = setup.paths.gsettings_schemas.as_ref()?;
        let file = setup.paths.gsettings_keyfile.as_ref()?;
        if !dir.join("gschemas.compiled").is_file() {
            return None;
        }
        let source = gio::SettingsSchemaSource::from_directory(dir, None, false).ok()?;
        (source.lookup(SCHEMA, false)?, Some(gio::keyfile_settings_backend_new(file.to_str()?, "/", None)))
    } else {
        (gio::SettingsSchemaSource::default()?.lookup(SCHEMA, true)?, None)
    };
    let typed = |key: &str, ty: &str| schema.has_key(key) && schema.key(key).value_type().as_str() == ty;
    if schema.path().is_none() || !typed(KEY_IGNORE, "s") || !typed(KEY_DIST, "b") {
        return None;
    }
    Some(gio::Settings::new_full(&schema, backend.as_ref(), None))
}
fn sync(setup: &super::paths::Setup) {
    // dconf writes asynchronously; the keyfile backend of test mode writes at once
    if !setup.test {
        gio::Settings::sync();
    }
}
/// Are Ubuntu's own release prompts held by us right now (the remembered record, and both keys still ours)? Only
/// Settings shows it, so only a `settings` host opens update-manager's settings (no dconf client in the shell hosts).
fn held(shared: &Shared) -> bool {
    if shared.ctx.kind != "settings" {
        return false;
    }
    let previous = shared.ctx.settings()["update"]["ubuntuHoldPrevious"].clone();
    let Some(codename) = previous["codename"].as_str() else { return false };
    update_manager(&shared.setup).is_some_and(|s| s.string(KEY_IGNORE).as_str() == codename && !s.boolean(KEY_DIST))
}
fn hold_failed(lang: Lang) -> ApiError {
    ApiError::tr("failed", lang, "update.release.err.ubuntuHold", NO_PARAMS)
}
/// Put back what we changed (only keys that still hold our values) and forget the record. Ok(true) when a record
/// was there. GW6 calls this when the verdict becomes ok/warn or the offered codename changes.
pub(crate) fn restore_hold(shared: &Shared) -> Result<bool, ()> {
    let previous = shared.ctx.settings()["update"]["ubuntuHoldPrevious"].clone();
    let Some(codename) = previous["codename"].as_str() else { return Ok(false) };
    let settings = update_manager(&shared.setup).ok_or(())?;
    if settings.string(KEY_IGNORE).as_str() == codename {
        match previous["ignore"].as_str() {
            Some(value) => settings.set_string(KEY_IGNORE, value).map_err(|_| ())?,
            None => settings.reset(KEY_IGNORE),
        }
    }
    if !settings.boolean(KEY_DIST) {
        match previous["checkDist"].as_bool() {
            Some(value) => settings.set_boolean(KEY_DIST, value).map_err(|_| ())?,
            None => settings.reset(KEY_DIST),
        }
    }
    sync(&shared.setup);
    shared.ctx.call("settings.set", &json!({"path":PREVIOUS,"value":null})).map_err(|_| ())?;
    Ok(true)
}
/// Hold Ubuntu's prompts for `codename`: remember the previous values (once per codename), then write ours.
pub(crate) fn hold_for(shared: &Shared, codename: &str) -> Result<(), ()> {
    let previous = shared.ctx.settings()["update"]["ubuntuHoldPrevious"].clone();
    if previous["codename"].as_str().is_some_and(|c| c != codename) {
        restore_hold(shared)?;
    }
    let settings = update_manager(&shared.setup).ok_or(())?;
    if shared.ctx.settings()["update"]["ubuntuHoldPrevious"]["codename"].as_str() != Some(codename) {
        let user = |key: &str| settings.user_value(key);
        let record = json!({"ignore":user(KEY_IGNORE).and_then(|v| v.str().map(str::to_owned)),
                            "checkDist":user(KEY_DIST).and_then(|v| v.get::<bool>()),"codename":codename});
        shared.ctx.call("settings.set", &json!({"path":PREVIOUS,"value":record})).map_err(|_| ())?;
    }
    settings.set_string(KEY_IGNORE, codename).map_err(|_| ())?;
    settings.set_boolean(KEY_DIST, false).map_err(|_| ())?;
    sync(&shared.setup);
    Ok(())
}
/// `update.release.ubuntuHold {on:bool}` → `{on, codename}` (module doc). Never in dry-run (`update.rs`).
pub(super) fn ubuntu_hold(shared: &Shared, args: &Value, lang: Lang) -> ApiResult {
    if args["on"] != true {
        let previous = shared.ctx.settings()["update"]["ubuntuHoldPrevious"]["codename"].clone();
        restore_hold(shared).map_err(|_| hold_failed(lang))?;
        return Ok(json!({"on":false,"codename":previous}));
    }
    let paths = &shared.setup.paths;
    let offered = reads_files(paths).then(|| detect(paths, &today()).offer).flatten();
    let codename = offered.map(|o| o.codename);
    let verdict = codename.as_deref().and_then(|c| compat::cached(shared, c)).map(|v| v["state"].clone());
    let Some(codename) = codename.filter(|_| matches!(verdict.as_ref().and_then(Value::as_str), Some("hold" | "blocked"))) else {
        return Err(ApiError::tr("not_held", lang, "update.release.err.notHeld", &json!({"product":super::product(lang)})));
    };
    hold_for(shared, &codename).map_err(|_| hold_failed(lang))?;
    Ok(json!({"on":true,"codename":codename}))
}

// ------------------------------------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::update::{
        Mode, Update,
        admin::tests::{default_file, fixture, install_helper, pkexec_argv, pkexec_stub, release_file, shared},
        paths::{Env, VAR_FIXTURE_ROOT, VAR_TOOL_DIR},
        tests::{Fixture, script, write},
    };
    use std::{
        sync::Arc,
        time::{Instant, SystemTime},
    };

    const TODAY: &str = "2026-11-06";
    const RESOLUTE: &str = "PRETTY_NAME=\"Ubuntu 26.04.1 LTS\"\nNAME=\"Ubuntu\"\nVERSION_ID=\"26.04\"\nVERSION=\"26.04.1 LTS (Resolute Raccoon)\"\nVERSION_CODENAME=resolute\nID=ubuntu\nUBUNTU_CODENAME=resolute\n";
    const QUESTING: &str = "NAME=\"Ubuntu\"\nVERSION_ID=\"25.10\"\nVERSION=\"25.10 (Questing Quokka)\"\nVERSION_CODENAME=questing\nUBUNTU_CODENAME=questing\n";

    /// This machine as design §1.2 records it (today): 26.04.1 LTS, Prompt=normal, a .d file without Prompt,
    /// distro-info, both meta-release copies without stonking, an empty stamp.
    fn machine(f: &Fixture) {
        write(&f.root.join("etc/os-release"), RESOLUTE);
        write(&release_file(f), &default_file());
        write(
            &f.root.join("etc/update-manager/release-upgrades.d/ubuntu-advantage-upgrades.cfg"),
            &fixture("etc/update-manager/release-upgrades.d/ubuntu-advantage-upgrades.cfg"),
        );
        write(&f.root.join("usr/share/distro-info/ubuntu.csv"), &fixture("distro-info/ubuntu.csv"));
        copy(f, "home/.cache/update-manager-core/meta-release", "today", 0);
        copy(f, "var/lib/update-manager/meta-release", "today", -3);
        write(&f.root.join("var/lib/ubuntu-release-upgrader/release-upgrade-available"), "");
        set_age(&f.root.join("var/lib/ubuntu-release-upgrader/release-upgrade-available"), 0);
    }
    /// A meta-release fixture at `rel` (under the fixture root), `days` old (negative: older).
    fn copy(f: &Fixture, rel: &str, name: &str, days: i64) {
        write(&f.root.join(rel), &fixture(&format!("meta-release/{name}")));
        set_age(&f.root.join(rel), days);
    }
    fn set_age(path: &Path, days: i64) {
        let base = SystemTime::now() - Duration::from_secs(3600);
        let at = if days >= 0 { base + Duration::from_secs(days as u64 * 60) } else { base - Duration::from_secs((-days) as u64 * 86400) };
        fs::File::options().write(true).open(path).unwrap().set_modified(at).unwrap();
    }
    fn paths(f: &Fixture) -> Paths {
        shared(f, "nl").setup.paths
    }

    #[test]
    fn the_prompt_follows_ubuntus_precedence_and_names_its_file() {
        assert_eq!(config_prompt(&default_file()), Some("normal".into()));
        assert_eq!(config_prompt("[DEFAULT]\n  prompt : LTS \n"), Some("lts".into()), "case, colon, spaces");
        assert_eq!(config_prompt("[Other]\nPrompt=lts\n[DEFAULT]\n# Prompt=never\n;Prompt=never\nX=1\n"), None, "comments and other sections");
        assert_eq!(config_prompt("[DEFAULT]\nPrompt=lts\nPrompt=never\n"), None, "a key twice: Ubuntu ignores the file");
        assert_eq!(config_prompt("[DEFAULT]\r\nPrompt=LTS\r\n"), Some("lts".into()), "CRLF");
        assert_eq!(config_prompt("[DEFAULT] ; main\nPrompt=never\n"), Some("never".into()), "a header with text after its ]");
        assert_eq!(config_prompt("[DEFAULT]\nX=1\n  Prompt=never\n"), None, "an indented line after a key continues that key");
        assert_eq!(config_prompt("[DEFAULT]\nPrompt=lts\n  more\nX=1\n"), Some("lts\nmore".into()), "a continuation is part of the value");
        assert_eq!(config_prompt("[DEFAULT]\nPrompt=lts # not a comment\n"), Some("lts # not a comment".into()), "no inline comments");
        assert_eq!(config_prompt("Prompt=lts\n"), None, "outside any section");
        for (raw, value) in [(Some("never"), "never"), (Some("no"), "never"), (Some("lts"), "lts"), (Some("normal"), "normal"), (Some("lts\nmore"), "normal"), (Some("yes"), "normal"), (None, "normal")] {
            assert_eq!(prompt_value(raw), value, "{raw:?}");
        }
        let f = Fixture::new("release-prompt");
        assert_eq!(read_prompt(&paths(&f)), None, "no /etc/update-manager");
        machine(&f);
        let main = "/etc/update-manager/release-upgrades".to_owned();
        assert_eq!(read_prompt(&paths(&f)), Some(Prompt { value: "normal", main: Some("normal".into()), file: main.clone(), overridden: None }));
        // .d files in byte order, the last Prompt wins
        let d = f.root.join("etc/update-manager/release-upgrades.d");
        write(&d.join("50-b.cfg"), "[DEFAULT]\nPrompt=never\n");
        write(&d.join("10-a.cfg"), "[DEFAULT]\nPrompt=lts\n");
        write(&d.join("90-c.cfg"), "[Sources]\nPrompt=normal\n");
        write(&d.join("99-d.conf"), "[DEFAULT]\nPrompt=normal\n");
        write(&d.join("60-e.cfg"), "[DEFAULT]\nPrompt=lts\nPrompt=lts\n");
        let p = read_prompt(&paths(&f)).unwrap();
        assert_eq!((p.value, p.main.as_deref(), p.file.as_str(), p.overridden.as_deref()),
                   ("never", Some("normal"), "/etc/update-manager/release-upgrades.d/50-b.cfg", Some("/etc/update-manager/release-upgrades.d/50-b.cfg")),
                   "60-e.cfg has a key twice: Ubuntu ignores it");
        fs::remove_dir_all(&d).unwrap();
        fs::remove_file(release_file(&f)).unwrap();
        assert_eq!(read_prompt(&paths(&f)), Some(Prompt { value: "normal", main: None, file: main, overridden: None }), "no file: Ubuntu's default");
    }

    /// The corpus of tests/update/fixtures/release/cfg: what Python's configparser reads from each file, recorded in
    /// expected.tsv (test_helpers.sh checks those verdicts against python3 and runs the helper on every file).
    #[test]
    fn config_prompt_reads_each_file_as_configparser_does() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/update/fixtures/release/cfg");
        let expected = fs::read_to_string(dir.join("expected.tsv")).unwrap();
        let mut cases = 0;
        for line in expected.lines().filter(|l| !l.starts_with('#') && !l.is_empty()) {
            let cells: Vec<&str> = line.split('\t').collect();
            let (name, verdict) = (cells[0], cells[1]);
            let text = fs::read_to_string(dir.join(format!("{name}.cfg"))).unwrap();
            let want = verdict.strip_prefix("value:").map(|v| v.replace("\\n", "\n"));
            assert_eq!(config_prompt(&text), want, "{name}: Ubuntu reads {verdict}");
            cases += 1;
        }
        assert!(cases >= 30, "{cases}");
    }

    #[test]
    fn meta_release_parses_like_metarelease_py() {
        let today = parse_meta_release(&fixture("meta-release/today")).unwrap();
        assert_eq!(today.iter().map(|d| d.dist.as_str()).collect::<Vec<_>>(), ["warty", "focal", "jammy", "noble", "oracular", "plucky", "questing", "resolute"]);
        let resolute = today.last().unwrap();
        assert_eq!((resolute.version.as_str(), resolute.name.as_deref(), resolute.date, resolute.supported),
                   ("26.04.1 LTS", Some("Resolute Raccoon"), [2026, 4, 23, 15, 0, 0], true));
        assert_eq!(today[0].date, [2004, 10, 20, 7, 28, 17], "3-letter month");
        assert_eq!(offer(&today, "resolute"), None, "today: nothing after resolute");
        assert_eq!(offer(&today, "questing").map(|d| d.dist.as_str()), Some("resolute"), "an unsupported current release still gets the offer");
        assert_eq!(offer(&today, "stonking"), None, "the current release must be listed");
        let s = parse_meta_release(&fixture("meta-release/stonking-supported")).unwrap();
        let o = offer(&s, "resolute").unwrap();
        assert_eq!((o.dist.as_str(), o.version.as_str(), o.broken.as_deref()), ("stonking", "26.10", None));
        assert_eq!(o.notes.as_deref(), Some("http://archive.ubuntu.com/ubuntu/dists/stonking/main/dist-upgrader-all/current/ReleaseAnnouncement.html"));
        assert_eq!(offer(&parse_meta_release(&fixture("meta-release/stonking-unsupported")).unwrap(), "resolute"), None, "Supported: 0");
        let broken = parse_meta_release(&fixture("meta-release/stonking-broken")).unwrap();
        assert_eq!(offer(&broken, "resolute").unwrap().broken.as_deref(), Some("Upgrades to Ubuntu 26.10 are on hold while a bug is fixed."));
        assert_eq!(parse_meta_release(&fixture("meta-release/broken-file")), None, "a stanza without Supported refuses the file");
        assert_eq!((parse_meta_release(""), parse_meta_release("\n\n# c\n")), (None, None), "no stanza: no copy");
        assert_eq!(offer(&parse_meta_release(&fixture("meta-release/today-lts")).unwrap(), "noble").map(|d| d.dist.as_str()), Some("resolute"));
        // dates as email.utils.parsedate reads them, folded fields
        assert_eq!(parse_date("Thu, 15 October 2026 12:00:00 UTC"), Some([2026, 10, 15, 12, 0, 0]));
        assert_eq!(parse_date("15 Oct 26 12:00 +0000"), Some([2026, 10, 15, 12, 0, 0]));
        assert_eq!(parse_date("Thu, October 15 2026 12:00:00"), Some([2026, 10, 15, 12, 0, 0]));
        assert_eq!(parse_date("Thu, 15 Oktober 2026"), None);
        let folded = parse_meta_release("Dist: a\nVersion: 1\nSupported: 1\nDate: 1 Jan 2020\nDescription: x\n y\n").unwrap();
        assert_eq!(folded[0].date, [2020, 1, 1, 0, 0, 0]);
        // the stamp: only the version, in any language
        assert_eq!(stamp_version(&fixture("stamp/new-release-en")).as_deref(), Some("26.10"));
        assert_eq!(stamp_version(&fixture("stamp/new-release-nl")).as_deref(), Some("26.10"));
        assert_eq!(stamp_version(&fixture("stamp/new-lts-en")).as_deref(), Some("28.04.1 LTS"));
        assert_eq!(stamp_version(&fixture("stamp/unsupported-en")), None);
        assert_eq!(stamp_version(""), None);
        assert_eq!(stamp_version("v126.10 and 2026.10.1"), None, "not inside a longer number");
    }

    #[test]
    fn distro_info_gives_dates_and_the_released_release() {
        let rows = parse_distro_info(&fixture("distro-info/ubuntu.csv"));
        let stonking = rows.iter().find(|r| r.series == "stonking").unwrap();
        assert_eq!((stonking.version.as_str(), stonking.release.as_deref(), stonking.eol.as_deref()), ("26.10", Some("2026-10-15"), Some("2027-07-15")));
        assert_eq!(months_between("2026-10-15", "2027-07-15"), Some(9));
        assert_eq!(months_between("2026-04-23", "2031-05-29"), Some(61));
        assert_eq!(months_between("2026-04-23", "2026-05-22"), Some(0));
        assert_eq!(parse_distro_info("series,version\nx,1\n"), vec![DistroRow { version: "1".into(), series: "x".into(), release: None, eol: None }], "by header name");
        assert_eq!(parse_distro_info(""), vec![]);
        assert_eq!(parse_os_release(RESOLUTE), Current { codename: Some("resolute".into()), version_id: Some("26.04".into()), lts: true });
        assert_eq!(parse_os_release(QUESTING).lts, false);
    }

    #[test]
    fn today_on_this_machine_is_released_but_not_offered() {
        let f = Fixture::new("release-today");
        machine(&f);
        let d = detect(&paths(&f), TODAY);
        assert_eq!(d.offer, None);
        assert_eq!(d.released, Some(("26.10".into(), "stonking".into())));
        assert_eq!(d.prompt.as_ref().map(|p| p.value), Some("normal"));
        assert!(d.current.lts);
        assert_eq!(d.checked_at, mtime_ms(&f.root.join("home/.cache/update-manager-core/meta-release")), "the newer copy: the user's");
        // before 26.10's release day there is nothing out; 27.04 is out only from 2027-04-22
        assert_eq!(detect(&paths(&f), "2026-10-14").released, None);
        assert_eq!(detect(&paths(&f), "2027-05-01").released, Some(("27.04".into(), "tenacious".into())));
        assert_eq!(detect(&paths(&f), "2027-07-16").released, Some(("27.04".into(), "tenacious".into())), "26.10 past its eol");
        // Prompt=lts on an LTS: only LTS versions count, and Ubuntu reads meta-release-lts
        let lts = default_file().replace("Prompt=normal", "Prompt=lts");
        write(&release_file(&f), &lts);
        copy(&f, "home/.cache/update-manager-core/meta-release", "stonking-supported", 0);
        let d = detect(&paths(&f), TODAY);
        assert_eq!((d.offer.clone(), d.released.clone(), d.checked_at), (None, None, None), "no meta-release-lts copy yet");
        copy(&f, "var/lib/update-manager/meta-release-lts", "today-lts", -1);
        assert_eq!(detect(&paths(&f), TODAY).checked_at, mtime_ms(&f.root.join("var/lib/update-manager/meta-release-lts")));
        // Prompt=never: nothing offered, whatever the copies say
        write(&release_file(&f), &default_file().replace("Prompt=normal", "Prompt=never"));
        assert_eq!(detect(&paths(&f), TODAY).offer, None);
    }

    #[test]
    fn the_newer_copy_wins_and_the_stamp_counts_only_when_newer() {
        let f = Fixture::new("release-offer");
        machine(&f);
        // root's copy (timer) is newer and lists stonking: offered, with distro-info's dates
        copy(&f, "var/lib/update-manager/meta-release", "stonking-supported", 0);
        set_age(&f.root.join("home/.cache/update-manager-core/meta-release"), -2);
        let o = detect(&paths(&f), TODAY).offer.unwrap();
        assert_eq!((o.version.as_str(), o.codename.as_str(), o.name.as_deref(), o.source), ("26.10", "stonking", Some("Stonking Stingray"), "cache"));
        assert_eq!((o.eol.as_deref(), o.support_months), (Some("2027-07-15"), Some(9)));
        // the user's copy newer again (no stonking): not offered
        set_age(&f.root.join("home/.cache/update-manager-core/meta-release"), 1);
        assert_eq!(detect(&paths(&f), TODAY).offer, None);
        // a newer copy that does not parse is skipped
        copy(&f, "home/.cache/update-manager-core/meta-release", "broken-file", 2);
        assert_eq!(detect(&paths(&f), TODAY).offer.map(|o| o.codename), Some("stonking".into()));
        // UpgradeBroken reaches the snapshot (GW6's ubuntuHold card)
        copy(&f, "home/.cache/update-manager-core/meta-release", "stonking-broken", 3);
        assert!(detect(&paths(&f), TODAY).offer.unwrap().broken.is_some());
        // the stamp: newer than the copy and a later version → offered by version, codename from distro-info
        copy(&f, "home/.cache/update-manager-core/meta-release", "today", 0);
        copy(&f, "var/lib/update-manager/meta-release", "today", -1);
        let stamp = f.root.join("var/lib/ubuntu-release-upgrader/release-upgrade-available");
        write(&stamp, &fixture("stamp/new-release-nl"));
        set_age(&stamp, 1);
        let o = detect(&paths(&f), TODAY).offer.unwrap();
        assert_eq!((o.version.as_str(), o.codename.as_str(), o.source, o.support_months), ("26.10", "stonking", "stamp", Some(9)));
        set_age(&stamp, -1);
        assert_eq!(detect(&paths(&f), TODAY).offer, None, "an older stamp is stale");
        write(&stamp, "New release '26.04' available.\n");
        assert_eq!(detect(&paths(&f), TODAY).offer, None, "not later than this release");
        write(&stamp, &fixture("stamp/current-point-en"));
        assert_eq!(stamp_version(&fixture("stamp/current-point-en")).as_deref(), Some("26.04.1 LTS"));
        assert_eq!(detect(&paths(&f), TODAY).offer, None, "a stale stamp naming this release's point version (written on 25.10)");
        // an empty newer copy is no copy: the older one that lists stonking still counts
        write(&stamp, "");
        copy(&f, "var/lib/update-manager/meta-release", "stonking-supported", 0);
        write(&f.root.join("home/.cache/update-manager-core/meta-release"), "");
        set_age(&f.root.join("home/.cache/update-manager-core/meta-release"), 2);
        assert_eq!(detect(&paths(&f), TODAY).offer.map(|o| o.codename), Some("stonking".into()));
    }

    #[test]
    fn lts_on_an_interim_release_behaves_as_normal() {
        let f = Fixture::new("release-interim");
        machine(&f);
        write(&f.root.join("etc/os-release"), QUESTING);
        write(&release_file(&f), &default_file().replace("Prompt=normal", "Prompt=lts"));
        let d = detect(&paths(&f), TODAY);
        assert!(!d.current.lts);
        assert_eq!(d.prompt.as_ref().map(|p| p.value), Some("lts"), "the file's value, for promptLtsInterim");
        let o = d.offer.unwrap();
        assert_eq!((o.version.as_str(), o.codename.as_str()), ("26.04.1 LTS", "resolute"), "meta-release, not -lts");
        assert_eq!(d.released, Some(("26.10".into(), "stonking".into())), "every version, not only LTS");
    }

    #[test]
    fn the_snapshot_release_object() {
        let f = Fixture::new("release-status");
        machine(&f);
        let shared = shared(&f, "nl");
        let r = status(&shared);
        assert_eq!(r["state"], "none");
        assert_eq!((r["offered"].clone(), r["released"].clone()), (json!(false), json!({"version":"26.10","codename":"stonking"})));
        assert_eq!((r["prompt"].clone(), r["promptFile"].clone(), r["override"].clone()), (json!("normal"), json!("/etc/update-manager/release-upgrades"), Value::Null));
        assert_eq!((r["currentLts"].clone(), r["helper"].clone(), r["ubuntuPromptHeld"].clone()), (json!(true), json!({"prompt":true,"attach":true}), json!(false)));
        assert_eq!((r["lastCheck"].clone(), r["checkError"].clone()), (Value::Null, Value::Null));
        // a page reads it through update.status; no program ran for any of it
        let service = Update::with_env(f.ctx("panel", false, "nl"), &Env::fixture(&[(VAR_TOOL_DIR, f.tools.to_str().unwrap()), (VAR_FIXTURE_ROOT, f.root.to_str().unwrap())], unsafe { libc::geteuid() }));
        assert_eq!(service.call("update.status", &json!({})).unwrap().unwrap()["release"]["released"]["version"], "26.10");
        service.shutdown();
        copy(&f, "var/lib/update-manager/meta-release", "stonking-supported", 1);
        let r = status(&shared);
        assert_eq!((r["offered"].clone(), r["version"].clone(), r["codename"].clone(), r["eol"].clone(), r["supportMonths"].clone()),
                   (json!(true), json!("26.10"), json!("stonking"), json!("2027-07-15"), json!(9)));
        assert_eq!(r["notesUrl"], "http://archive.ubuntu.com/ubuntu/dists/stonking/main/dist-upgrader-all/current/ReleaseAnnouncement.html");
        assert_eq!(shared.setup.tools.runs(), 0);
        assert_eq!(f.stub_log(), "");
        // a unit test without a fixture root reads no release file at all
        let bare = Env::fixture(&[(VAR_TOOL_DIR, f.tools.to_str().unwrap())], unsafe { libc::geteuid() });
        let setup = crate::services::update::paths::resolve(&bare).unwrap();
        assert!(!reads_files(&setup.paths));
    }

    /// A check-new-release stub: logs its argv and environment, writes `copy` (a fixture) as the user's
    /// meta-release copy when given, exits `rc` after `sleep` seconds of busy waiting (builtins only).
    fn check_stub(f: &Fixture, rc: i32, copy: Option<&str>) {
        let body = copy.map(|name| fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/update/fixtures/release/meta-release").join(name)).unwrap()).unwrap_or_default();
        fs::write(f.base.join("check.copy"), body).unwrap();
        script(
            &f.tools.join("check-new-release"),
            &format!(
                "echo \"check-new-release $* LANG=$LANG XDG_CACHE_HOME=$XDG_CACHE_HOME HOME=$HOME\" >> '{log}'\n\
                 if [ -s '{data}' ]; then while IFS= read -r line; do echo \"$line\"; done < '{data}' > \"$XDG_CACHE_HOME/update-manager-core/meta-release\"; fi\n\
                 exit {rc}\n",
                log = f.log.display(),
                data = f.base.join("check.copy").display(),
            ),
        );
    }

    #[test]
    fn check_new_release_runs_only_through_the_stub_and_only_on_the_click_path() {
        let f = Fixture::new("release-check");
        machine(&f);
        check_stub(&f, 0, Some("stonking-supported"));
        let shared = shared(&f, "nl");
        // the read paths never start it
        for _ in 0..3 {
            status(&shared);
        }
        assert_eq!(shared.setup.tools.runs(), 0);
        let record = run_check(&shared, 1_000);
        assert_eq!((record["lastRun"].clone(), record["rc"].clone(), record["error"].clone(), record["newRelease"].clone()), (json!(1_000), json!(0), Value::Null, json!(true)));
        let cache = f.root.join("home/.cache");
        assert_eq!(f.stub_log(), format!("check-new-release -q LANG=C XDG_CACHE_HOME={} HOME={}\n", cache.display(), f.root.join("home").display()));
        // the copy it wrote is read at once
        assert_eq!(status(&shared)["codename"], "stonking");
        // exit 1 = none; anything else is the check error, shown in the user's language
        check_stub(&f, 1, None);
        assert_eq!(run_check(&shared, 2_000)["error"], Value::Null);
        check_stub(&f, 2, None);
        assert_eq!(run_check(&shared, 3_000)["error"]["key"], "update.release.err.checkFailed");
        assert_eq!(status(&shared)["checkError"]["message"], "Controleren op een nieuwe Ubuntu-versie is mislukt.");
        assert_eq!(status(&shared)["lastCheck"], 3_000);
        // no stub: a seam error, never the real program
        fs::remove_file(f.tools.join("check-new-release")).unwrap();
        assert_eq!(run_check(&shared, 4_000)["error"]["key"], "update.release.err.checkFailed");
        assert_eq!(shared.setup.tools.runs(), 3);
    }

    #[test]
    fn a_click_checks_at_most_daily_and_never_while_metered_offline_or_never() {
        let f = Fixture::new("release-click");
        machine(&f);
        check_stub(&f, 1, None);
        let shared = shared(&f, "nl");
        let wait_done = || {
            let started = Instant::now();
            while RUNNING.lock().unwrap().contains(&shared.ctx.cache_dir) && started.elapsed() < Duration::from_secs(10) {
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        let changed = Arc::new(Mutex::new(Vec::new()));
        let seen = changed.clone();
        shared.ctx.events.on("update.changed", Arc::new(move |d: &Value| seen.lock().unwrap().push(d.clone())));
        assert_eq!(check(&shared, Lang::Nl).unwrap(), json!({"started":true}));
        wait_done();
        assert_eq!(f.stub_log().lines().count(), 1);
        assert_eq!(check(&shared, Lang::Nl).unwrap(), json!({"started":false,"skipped":"recent"}));
        // after a failed run a click may try again after an hour (not after a day)
        let ok_record = shared.store.read(RELEASE).unwrap();
        let mut failed = ok_record.clone();
        failed["error"] = json!({"code":"check_failed","key":"update.release.err.checkFailed"});
        failed["lastRun"] = json!(store::now_ms() - CHECK_RETRY_MS + 60_000);
        shared.store.write(RELEASE, &failed).unwrap();
        assert_eq!(check(&shared, Lang::Nl).unwrap()["skipped"], "recent");
        failed["lastRun"] = json!(store::now_ms() - CHECK_RETRY_MS - 1);
        shared.store.write(RELEASE, &failed).unwrap();
        assert_eq!(check(&shared, Lang::Nl).unwrap()["started"], true);
        wait_done();
        assert_eq!(f.stub_log().lines().count(), 2);
        assert_eq!(shared.store.read(RELEASE).unwrap()["error"], Value::Null, "the stub answered 1: no error");
        let mut ok_hour = ok_record;
        ok_hour["lastRun"] = json!(store::now_ms() - CHECK_RETRY_MS - 1);
        shared.store.write(RELEASE, &ok_hour).unwrap();
        assert_eq!(check(&shared, Lang::Nl).unwrap()["skipped"], "recent", "a good run still counts for a day");
        // more than a day later it runs again
        let mut record = shared.store.read(RELEASE).unwrap();
        record["lastRun"] = json!(store::now_ms() - CHECK_EVERY_MS - 1);
        shared.store.write(RELEASE, &record).unwrap();
        // metered (the snapshot's network): only with update.metered on
        shared.store.write(store::SNAPSHOT, &json!({"network":{"state":"full","online":true,"metered":true}})).unwrap();
        assert_eq!(check(&shared, Lang::Nl).unwrap()["skipped"], "metered");
        shared.ctx.call("settings.set", &json!({"path":"update.metered","value":true})).unwrap();
        assert_eq!(check(&shared, Lang::Nl).unwrap()["started"], true);
        wait_done();
        assert_eq!(f.stub_log().lines().count(), 3);
        shared.store.write(store::SNAPSHOT, &json!({"network":{"state":"none","online":false,"metered":false}})).unwrap();
        record["lastRun"] = json!(0);
        shared.store.write(RELEASE, &record).unwrap();
        assert_eq!(check(&shared, Lang::Nl).unwrap()["skipped"], "offline");
        shared.store.write(store::SNAPSHOT, &json!({})).unwrap();
        write(&release_file(&f), &default_file().replace("Prompt=normal", "Prompt=never"));
        assert_eq!(check(&shared, Lang::Nl).unwrap()["skipped"], "never");
        assert_eq!(f.stub_log().lines().count(), 3);
        // each finished check told the pages (rev moves past the snapshot's)
        let events = changed.lock().unwrap().clone();
        assert!(events.len() >= 2 && events.iter().all(|e| e["rev"].as_u64() > Some(0)), "{events:?}");
        // through the service: a dry Settings host answers without a program, a panel is not the owner
        let env = Env::fixture(&[(VAR_TOOL_DIR, f.tools.to_str().unwrap()), (VAR_FIXTURE_ROOT, f.root.to_str().unwrap())], unsafe { libc::geteuid() });
        let dry = Update::with_env(f.ctx("settings", true, "nl"), &env);
        assert_eq!(dry.mode(), Mode::Dry);
        assert_eq!(dry.call("update.release.check", &json!({})).unwrap().unwrap()["dryRun"], true);
        dry.shutdown();
        let panel = Update::with_env(f.ctx("panel", false, "nl"), &env);
        assert_eq!(panel.call("update.release.check", &json!({})).unwrap().unwrap_err().code, "not_here");
        panel.shutdown();
        assert_eq!(f.stub_log().lines().count(), 3);
    }

    #[test]
    fn set_prompt_changes_only_the_prompt_line_and_asks_no_password_for_nothing() {
        let f = Fixture::new("release-set");
        machine(&f);
        install_helper(&f);
        pkexec_stub(&f, "answer");
        let shared = shared(&f, "en");
        let changed = Arc::new(Mutex::new(0));
        let count = changed.clone();
        shared.ctx.events.on("update.changed", Arc::new(move |_| *count.lock().unwrap() += 1));
        // the same value: no pkexec
        assert_eq!(set_prompt(&shared, &json!({"prompt":"normal"}), Lang::En).unwrap(), json!({"prompt":"normal","override":null,"changed":false}));
        assert_eq!(shared.setup.tools.runs(), 0);
        let lts = set_prompt(&shared, &json!({"prompt":"lts"}), Lang::En).unwrap();
        assert_eq!(lts, json!({"prompt":"lts","override":null,"changed":true}));
        assert_eq!(fs::read_to_string(release_file(&f)).unwrap(), default_file().replace("Prompt=normal\n", "Prompt=lts\n"));
        assert_eq!(pkexec_argv(&f)[1..], ["set-prompt", "lts"]);
        assert_eq!(status(&shared)["prompt"], "lts");
        assert_eq!(*changed.lock().unwrap(), 1);
        // a .d file decides: nothing is asked or changed, the page gets the override
        write(&f.root.join("etc/update-manager/release-upgrades.d/50-company.cfg"), "[DEFAULT]\nPrompt=never\n");
        let runs = shared.setup.tools.runs();
        assert_eq!(set_prompt(&shared, &json!({"prompt":"normal"}), Lang::En).unwrap(),
                   json!({"prompt":"never","override":"/etc/update-manager/release-upgrades.d/50-company.cfg","changed":false}));
        assert_eq!(shared.setup.tools.runs(), runs);
        fs::remove_file(f.root.join("etc/update-manager/release-upgrades.d/50-company.cfg")).unwrap();
        // a dismissed dialog: the design's text, the file as it was
        pkexec_stub(&f, "126");
        let error = set_prompt(&shared, &json!({"prompt":"never"}), Lang::En).unwrap_err();
        assert_eq!((error.code.as_str(), error.key.as_deref()), ("auth_cancelled", Some("update.release.err.noPassword")));
        assert!(fs::read_to_string(release_file(&f)).unwrap().contains("\nPrompt=lts\n"));
        assert!(set_prompt(&shared, &json!({"prompt":"sometimes"}), Lang::En).is_err());
        // a CRLF file (Ubuntu reads it): the helper keeps the line ends and Ubuntu reads the new value
        write(&release_file(&f), &default_file().replace('\n', "\r\n"));
        pkexec_stub(&f, "answer");
        assert_eq!(set_prompt(&shared, &json!({"prompt":"lts"}), Lang::En).unwrap(), json!({"prompt":"lts","override":null,"changed":true}));
        assert_eq!(fs::read_to_string(release_file(&f)).unwrap(), default_file().replace("Prompt=normal", "Prompt=lts").replace('\n', "\r\n"));
        // a file Ubuntu ignores (a key twice in another section): the helper refuses, the file stays as it was
        let refused = format!("{}[Sources]\nA=1\nA=2\n", default_file());
        write(&release_file(&f), &refused);
        let error = set_prompt(&shared, &json!({"prompt":"lts"}), Lang::En).unwrap_err();
        assert_eq!((error.code.as_str(), error.message.as_str()), ("failed", "The setting for new Ubuntu versions couldn’t be saved."));
        assert_eq!(fs::read_to_string(release_file(&f)).unwrap(), refused);
        // a helper that says @@ok without the file saying it: an error, never a silent no-op
        write(&release_file(&f), &default_file());
        script(&f.tools.join("release-upgrades-helper"), "echo '@@ok prompt=lts'\n");
        assert_eq!(set_prompt(&shared, &json!({"prompt":"lts"}), Lang::En).unwrap_err().key.as_deref(), Some("update.release.err.promptFailed"));
        assert_eq!(fs::read_to_string(release_file(&f)).unwrap(), default_file());
        // without the directory there is nothing to set
        fs::remove_dir_all(f.root.join("etc/update-manager")).unwrap();
        assert_eq!(set_prompt(&shared, &json!({"prompt":"lts"}), Lang::En).unwrap_err().key.as_deref(), Some("update.release.err.promptFailed"));
    }

    #[test]
    fn set_prompt_through_the_service_never_runs_in_dry_run_or_while_upgrading() {
        let f = Fixture::new("release-set-service");
        machine(&f);
        install_helper(&f);
        pkexec_stub(&f, "answer");
        let env = Env::fixture(&[(VAR_TOOL_DIR, f.tools.to_str().unwrap()), (VAR_FIXTURE_ROOT, f.root.to_str().unwrap())], unsafe { libc::geteuid() });
        let dry = Update::with_env(f.ctx("settings", true, "nl"), &env);
        assert_eq!(dry.call("update.release.setPrompt", &json!({"prompt":"lts"})).unwrap().unwrap(), json!({"dryRun":true}));
        assert_eq!(dry.call("update.release.setPrompt", &json!({"prompt":"often"})).unwrap().unwrap_err().code, "bad_args");
        dry.shutdown();
        write(&f.root.join("proc/777/cmdline"), "python3\0do-release-upgrade\0");
        let owner = Update::with_env(f.ctx("settings", false, "nl"), &env);
        let refused = owner.call("update.release.setPrompt", &json!({"prompt":"lts"})).unwrap().unwrap_err();
        assert_eq!(refused.key.as_deref(), Some("update.guard.upgrade"));
        owner.shutdown();
        assert_eq!(fs::read_to_string(release_file(&f)).unwrap(), default_file());
        assert_eq!(f.stub_log(), "");
    }

    fn install_schema(f: &Fixture) {
        let dir = f.root.join("usr/share/glib-2.0/schemas");
        fs::create_dir_all(&dir).unwrap();
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/update/fixtures/release/schemas");
        for name in ["gschemas.compiled", "com.ubuntu.update-manager.gschema.xml"] {
            fs::copy(src.join(name), dir.join(name)).unwrap();
        }
    }
    fn keyfile(f: &Fixture) -> String {
        fs::read_to_string(f.root.join("home/.config/glib-2.0/settings/keyfile")).unwrap_or_default()
    }
    fn set_keyfile(f: &Fixture, text: &str) {
        write(&f.root.join("home/.config/glib-2.0/settings/keyfile"), text);
    }

    #[test]
    fn ubuntu_hold_round_trips_both_keys_and_restores_only_our_values() {
        let f = Fixture::new("release-hold");
        machine(&f);
        install_schema(&f);
        let shared = shared(&f, "nl");
        let previous = |s: &Shared| s.ctx.settings()["update"]["ubuntuHoldPrevious"].clone();
        // defaults (no user values): on writes ours and remembers "no user value"
        hold_for(&shared, "stonking").unwrap();
        let text = keyfile(&f);
        assert!(text.contains("[apps/update-manager]") && text.contains("check-new-release-ignore='stonking'") && text.contains("check-dist-upgrades=false"), "{text}");
        assert_eq!(previous(&shared), json!({"ignore":null,"checkDist":null,"codename":"stonking"}));
        assert!(held(&shared));
        assert_eq!(status(&shared)["ubuntuPromptHeld"], true);
        // on again: the record stays the first one
        hold_for(&shared, "stonking").unwrap();
        assert_eq!(previous(&shared)["ignore"], Value::Null);
        // off: both keys back to their defaults, the record gone
        assert_eq!(ubuntu_hold(&shared, &json!({"on":false}), Lang::Nl).unwrap(), json!({"on":false,"codename":"stonking"}));
        assert!(!keyfile(&f).contains("check-new-release-ignore=") && !keyfile(&f).contains("check-dist-upgrades="), "{}", keyfile(&f));
        assert_eq!(previous(&shared), Value::Null);
        assert!(!held(&shared));
        // the user's own values survive a round trip
        set_keyfile(&f, "[apps/update-manager]\ncheck-new-release-ignore='questing'\ncheck-dist-upgrades=true\n");
        hold_for(&shared, "stonking").unwrap();
        assert_eq!(previous(&shared), json!({"ignore":"questing","checkDist":true,"codename":"stonking"}));
        restore_hold(&shared).unwrap();
        let back = update_manager(&shared.setup).unwrap();
        assert_eq!((back.string(KEY_IGNORE).to_string(), back.boolean(KEY_DIST)), ("questing".into(), true));
        // a key the user changed after our hold stays the user's; the other one is restored
        hold_for(&shared, "stonking").unwrap();
        update_manager(&shared.setup).unwrap().set_string(KEY_IGNORE, "tenacious").unwrap();
        restore_hold(&shared).unwrap();
        let back = update_manager(&shared.setup).unwrap();
        assert_eq!((back.string(KEY_IGNORE).to_string(), back.boolean(KEY_DIST)), ("tenacious".into(), true));
        // a new codename restores the old hold first, then holds the new one with a fresh record
        set_keyfile(&f, "");
        hold_for(&shared, "stonking").unwrap();
        hold_for(&shared, "tenacious").unwrap();
        assert_eq!(previous(&shared), json!({"ignore":null,"checkDist":null,"codename":"tenacious"}));
        assert_eq!(update_manager(&shared.setup).unwrap().string(KEY_IGNORE).as_str(), "tenacious");
        assert_eq!(ubuntu_hold(&shared, &json!({"on":false}), Lang::Nl).unwrap()["codename"], "tenacious");
        assert!(!keyfile(&f).contains('='), "{}", keyfile(&f));
        // off without a record changes nothing
        assert_eq!(ubuntu_hold(&shared, &json!({"on":false}), Lang::Nl).unwrap(), json!({"on":false,"codename":null}));
        assert_eq!(shared.setup.tools.runs(), 0);
    }

    #[test]
    fn ubuntu_hold_on_needs_an_offer_on_hold_and_a_schema() {
        let f = Fixture::new("release-hold-guard");
        machine(&f);
        install_schema(&f);
        let shared = shared(&f, "en");
        // GW6: nothing offered (today), and an offered release whose verdict is not hold/blocked (compat::cached: a
        // releases.json that marks stonking tested with GNOME 50, the extension's major, and no check run yet)
        write(&f.root.join("releases.json"), r#"{"version":1,"extension":"win11-gnome@win11shell.local","releases":{"stonking":{"version":"26.10","gnomeShell":"50","status":"tested"}}}"#);
        write(&f.root.join("home/.local/share/gnome-shell/extensions/win11-gnome@win11shell.local/metadata.json"), r#"{"shell-version":["50"],"version":4}"#);
        for meta in ["today", "stonking-supported"] {
            copy(&f, "var/lib/update-manager/meta-release", meta, 1);
            let error = ubuntu_hold(&shared, &json!({"on":true}), Lang::En).unwrap_err();
            assert_eq!((error.code.as_str(), error.key.as_deref()), ("not_held", Some("update.release.err.notHeld")));
            assert_eq!(error.message, "This is only possible while WinPoint isn’t ready for the new Ubuntu version yet.");
        }
        assert_eq!(keyfile(&f), "");
        // GW6: the embedded releases.json (stonking untested, GNOME 51) is the cheap hold: the postponement is allowed,
        // and a Settings host's status gives it back once another release is offered (release::status, §4.12)
        fs::remove_file(f.root.join("releases.json")).unwrap();
        assert_eq!(ubuntu_hold(&shared, &json!({"on":true}), Lang::En).unwrap(), json!({"on":true,"codename":"stonking"}));
        assert_eq!(status(&shared)["state"], "hold");
        assert_eq!(status(&shared)["ubuntuPromptHeld"], true);
        copy(&f, "var/lib/update-manager/meta-release", "today", 2);
        assert_eq!(status(&shared)["ubuntuPromptHeld"], true, "no offer at all is no other release: the hold stays");
        let mut tenacious = fixture("meta-release/stonking-supported").replace("Dist: stonking", "Dist: tenacious").replace("Version: 26.10", "Version: 27.04");
        tenacious = tenacious.replace("Date: Thu, 15 October 2026", "Date: Thu, 22 April 2027");
        write(&f.root.join("var/lib/update-manager/meta-release"), &tenacious);
        let r = status(&shared);
        assert_eq!((r["codename"].clone(), r["ubuntuPromptHeld"].clone()), (json!("tenacious"), json!(false)), "{r}");
        assert_eq!(shared.ctx.settings()["update"]["ubuntuHoldPrevious"], Value::Null);
        assert_eq!(keyfile(&f).matches('=').count(), 0, "{}", keyfile(&f));
        set_keyfile(&f, ""); // the checks below start from an empty keyfile, as before GW6
        // GW6 end
        // no schema: the design's text
        fs::remove_dir_all(f.root.join("usr/share/glib-2.0/schemas")).unwrap();
        shared.ctx.call("settings.set", &json!({"path":PREVIOUS,"value":{"ignore":null,"checkDist":null,"codename":"stonking"}})).unwrap();
        let error = ubuntu_hold(&shared, &json!({"on":false}), Lang::En).unwrap_err();
        assert_eq!((error.code.as_str(), error.message.as_str()), ("failed", "Ubuntu’s own prompt couldn’t be postponed."));
        assert!(update_manager(&shared.setup).is_none());
        // test mode without a fixture root never opens a settings backend (never dconf)
        let bare = Env::fixture(&[(VAR_TOOL_DIR, f.tools.to_str().unwrap())], unsafe { libc::geteuid() });
        assert!(update_manager(&crate::services::update::paths::resolve(&bare).unwrap()).is_none());
        // through the service: dry-run answers without writing
        install_schema(&f);
        let env = Env::fixture(&[(VAR_TOOL_DIR, f.tools.to_str().unwrap()), (VAR_FIXTURE_ROOT, f.root.to_str().unwrap())], unsafe { libc::geteuid() });
        let dry = Update::with_env(f.ctx("settings", true, "en"), &env);
        assert_eq!(dry.call("update.release.ubuntuHold", &json!({"on":true})).unwrap().unwrap(), json!({"dryRun":true}));
        dry.shutdown();
        assert_eq!(keyfile(&f), "");
    }
}
