//! The pure part of the update service: PackageKit enums, row grouping, phase derivation, progress throttling,
//! error mapping and the release-wrapper outcome (design §3.2, §3.3, §4.4–§4.6, §6.4). No I/O here.
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

/// PackageKit enums, pinned from the local PackageKitGlib 1.3.4 typelib (design §4.6; GW1's
/// `tests/update/enums.json` compares them with the typelib).
#[allow(dead_code)] // the full table of design §4.6; not every role is a call of ours
pub(crate) mod role {
    pub const GET_DETAILS: u32 = 3;
    pub const GET_UPDATE_DETAIL: u32 = 8;
    pub const GET_UPDATES: u32 = 9;
    pub const INSTALL_PACKAGES: u32 = 11;
    pub const REFRESH_CACHE: u32 = 13;
    pub const REMOVE_PACKAGES: u32 = 14;
    pub const UPDATE_PACKAGES: u32 = 22;
    pub const GET_OLD_TRANSACTIONS: u32 = 28;
    pub const REPAIR_SYSTEM: u32 = 29;
}
pub(crate) mod status {
    pub const WAIT: u32 = 1;
    pub const SETUP: u32 = 2;
    pub const WAITING_FOR_LOCK: u32 = 30;
    pub const WAITING_FOR_AUTH: u32 = 31;
    /// download phase (design §4.5 step 2)
    pub const DOWNLOADING: [u32; 14] = [3, 4, 5, 7, 8, 13, 14, 20, 21, 22, 23, 24, 25, 27];
    /// commit phase: remove, install, update, cleanup, obsolete, test-commit, commit, run-hook
    pub const COMMITTING: [u32; 8] = [6, 9, 10, 11, 12, 15, 16, 36];
}
pub(crate) mod info {
    pub const BLOCKED: u32 = 9;
    pub const IMPORTANT: u32 = 7;
    pub const SECURITY: u32 = 8;
    pub const CRITICAL: u32 = 26;
    pub const DOWNLOADING: u32 = 10;
    pub const UPDATING: u32 = 11;
    pub const INSTALLING: u32 = 12;
    pub const FINISHED: u32 = 18;
    /// confirm-worthy in a simulate: removing, obsoleting, downgrading, remove, obsolete, downgrade
    pub const CONFIRM: [u32; 6] = [13, 15, 20, 28, 29, 30];
}
pub(crate) mod exit {
    pub const SUCCESS: u32 = 1;
    pub const FAILED: u32 = 2;
    pub const CANCELLED: u32 = 3;
    pub const NEED_UNTRUSTED: u32 = 8;
    pub const CANCELLED_PRIORITY: u32 = 9;
    pub const REPAIR_REQUIRED: u32 = 11;
}
/// RequireRestart types that need a reboot or a new session (application 2 is ignored, design §4.8).
pub(crate) const RESTART_TYPES: [u32; 4] = [3, 4, 5, 6];
/// GetUpdates filter "none" as a bitfield.
pub(crate) const FILTER_NONE: u64 = 1 << 1;
/// Transaction flags (bit numbers: only-trusted 1, simulate 2). Callers never pass flags.
pub(crate) const FLAG_ONLY_TRUSTED: u64 = 1 << 1;
pub(crate) const FLAG_SIMULATE: u64 = 1 << 2;
/// PackageKit error 27 no-packages-to-update counts as success.
pub(crate) const ERROR_NO_PACKAGES_TO_UPDATE: u32 = 27;
/// PackageKit errors 2/37/43: network, or a source warning for a non-Ubuntu source.
pub(crate) const NETWORK_ERRORS: [u32; 3] = [2, 37, 43];

/// The page's phase (design §4.3 snapshot, §4.5 derivation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Idle,
    Checking,
    Available,
    Downloading,
    Installing,
    Waiting,
    Foreign,
    RestartRequired,
    Error,
    Unsupported,
    Offline,
    RepairRequired,
    ReleaseRunning,
}
impl Phase {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Phase::Idle => "idle",
            Phase::Checking => "checking",
            Phase::Available => "available",
            Phase::Downloading => "downloading",
            Phase::Installing => "installing",
            Phase::Waiting => "waiting",
            Phase::Foreign => "foreign",
            Phase::RestartRequired => "restartRequired",
            Phase::Error => "error",
            Phase::Unsupported => "unsupported",
            Phase::Offline => "offline",
            Phase::RepairRequired => "repairRequired",
            Phase::ReleaseRunning => "releaseRunning",
        }
    }
}

/// The stage of our own running job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stage {
    Waiting,
    Downloading,
    Installing,
}
/// Design §4.5 step 2: lock/auth/wait/setup before the download → waiting; the simulate step and the download
/// statuses → downloading; the commit statuses or `committing` → installing. A lock or auth wait (30, 31) is
/// `waiting` in the simulate step and during a confirm too (review F13), so H07/H08 can show there.
pub(crate) fn stage(status: u32, committing: bool, simulating: bool) -> Stage {
    if committing || status::COMMITTING.contains(&status) {
        return Stage::Installing;
    }
    if [status::WAITING_FOR_LOCK, status::WAITING_FOR_AUTH].contains(&status) {
        return Stage::Waiting;
    }
    if simulating || status::DOWNLOADING.contains(&status) {
        return Stage::Downloading;
    }
    if [status::WAIT, status::SETUP].contains(&status) {
        return Stage::Waiting;
    }
    Stage::Downloading
}

/// Everything the phase depends on (design §4.5).
#[derive(Clone, Debug, Default)]
pub(crate) struct PhaseInput {
    pub unsupported: bool,
    pub release_running: bool,
    pub job: Option<Stage>,
    pub foreign: bool,
    pub repair_required: bool,
    pub restart_required: bool,
    pub error: bool,
    pub offline: bool,
    pub checking: bool,
    pub available: usize,
}
/// First match wins: unsupported → our running job → releaseRunning → foreign → repairRequired →
/// restartRequired → error → offline → checking → available → idle.
pub(crate) fn derive_phase(i: &PhaseInput) -> Phase {
    if i.unsupported {
        return Phase::Unsupported;
    }
    if let Some(stage) = i.job {
        return match stage {
            Stage::Waiting => Phase::Waiting,
            Stage::Downloading => Phase::Downloading,
            Stage::Installing => Phase::Installing,
        };
    }
    if i.release_running {
        Phase::ReleaseRunning
    } else if i.foreign {
        Phase::Foreign
    } else if i.repair_required {
        Phase::RepairRequired
    } else if i.restart_required {
        Phase::RestartRequired
    } else if i.error {
        Phase::Error
    } else if i.offline {
        Phase::Offline
    } else if i.checking {
        Phase::Checking
    } else if i.available > 0 {
        Phase::Available
    } else {
        Phase::Idle
    }
}

/// One package of GetUpdates with our classification (design §4.3 `packages`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Pkg {
    pub id: String,
    pub name: String,
    pub arch: String,
    pub data: String,
    pub to: String,
    pub from: Option<String>,
    pub info: u32,
    pub summary: String,
    pub phased: bool,
    pub pup: Option<u32>,
    pub held: bool,
    pub size: Option<u64>,
}
impl Pkg {
    /// From a PackageKit `Package` signal: `name;version;arch;data`. None for an id that is not of that shape.
    pub(crate) fn from_signal(info: u32, id: &str, summary: &str) -> Option<Self> {
        let parts: Vec<&str> = id.split(';').collect();
        if parts.len() != 4 || id.len() > 512 || !valid_name(parts[0]) || parts[1].is_empty() {
            return None;
        }
        Some(Self {
            id: id.to_owned(),
            name: parts[0].to_owned(),
            to: parts[1].to_owned(),
            arch: parts[2].to_owned(),
            data: parts[3].to_owned(),
            info,
            held: info == info::BLOCKED,
            summary: summary.chars().take(300).collect(),
            ..Self::default()
        })
    }
    pub(crate) fn security(&self) -> bool {
        [info::SECURITY, info::IMPORTANT, info::CRITICAL].contains(&self.info) || self.data.contains("-security")
    }
    /// In a row and in the default install set: neither phased nor held.
    pub(crate) fn offered(&self) -> bool {
        !self.phased && !self.held
    }
    pub(crate) fn json(&self) -> Value {
        json!({"id":self.id,"name":self.name,"from":self.from,"to":self.to,"info":self.info,"summary":self.summary,
               "row":row_kind(&self.name).unwrap_or(if self.security() {"security"} else {"other"}),
               "phased":self.phased,"pup":self.pup,"held":self.held,"size":self.size})
    }
    pub(crate) fn from_json(v: &Value) -> Option<Self> {
        let mut pkg = Self::from_signal(
            v["info"].as_u64().and_then(|n| u32::try_from(n).ok())?,
            v["id"].as_str()?,
            v["summary"].as_str().unwrap_or(""),
        )?;
        pkg.from = v["from"].as_str().map(str::to_owned);
        pkg.phased = v["phased"] == true;
        pkg.held = v["held"] == true;
        pkg.pup = v["pup"].as_u64().and_then(|n| u32::try_from(n).ok());
        pkg.size = v["size"].as_u64();
        Some(pkg)
    }
}

/// A Debian package name as apt accepts it (also the only names the phasing classifier ever receives).
pub(crate) fn valid_name(name: &str) -> bool {
    let b = name.as_bytes();
    (2..=128).contains(&b.len())
        && b[0].is_ascii_alphanumeric()
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"+-.".contains(c))
}
pub(crate) fn valid_arch(arch: &str) -> bool {
    (1..=32).contains(&arch.len()) && arch.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

/// The component row of a package (design §3.3); None = security or other.
pub(crate) fn row_kind(name: &str) -> Option<&'static str> {
    let starts = |prefixes: &[&str]| prefixes.iter().any(|p| name.starts_with(p));
    if starts(&["gnome-shell", "mutter", "libmutter-", "gir1.2-mutter-"]) {
        Some("gnome")
    } else if starts(&["libwebkit2gtk-4.1-", "libjavascriptcoregtk-4.1-"])
        || ["gir1.2-webkit2-4.1", "gir1.2-javascriptcoregtk-4.1"].contains(&name)
    {
        Some("webkit")
    } else if starts(&["libgtk-3-"]) || name == "gir1.2-gtk-3.0" {
        Some("gtk")
    } else if starts(&["libsoup-3.0-"]) || name == "gir1.2-soup-3.0" {
        Some("soup")
    } else if starts(&["libglib2.0-"]) {
        Some("glib")
    } else if starts(&["linux-image-", "linux-modules-", "linux-headers-", "linux-generic"]) || name == "linux-base" {
        Some("kernel")
    } else {
        None
    }
}
/// The row order of design §3.3.
pub(crate) const ROW_ORDER: [&str; 8] = ["security", "gnome", "webkit", "gtk", "soup", "glib", "kernel", "other"];
/// The package whose version names a component row.
fn primary(kind: &str) -> &'static [&'static str] {
    match kind {
        "gnome" => &["gnome-shell"],
        "webkit" => &["libwebkit2gtk-4.1-0"],
        "gtk" => &["libgtk-3-0t64", "libgtk-3-0"],
        "soup" => &["libsoup-3.0-0"],
        "glib" => &["libglib2.0-0t64", "libglib2.0-0"],
        "kernel" => &["linux-image-"],
        _ => &[],
    }
}
/// The upstream part of a Debian version: no epoch, no Debian revision ("1:2.52.7-0ubuntu1" → "2.52.7").
pub(crate) fn upstream(version: &str) -> String {
    let no_epoch = version.split_once(':').map_or(version, |(_, rest)| rest);
    no_epoch.rsplit_once('-').map_or(no_epoch, |(up, _)| up).to_owned()
}
/// os-release `VERSION` without a trailing " (Codename)": "26.04.1 LTS (Resolute Raccoon)" → "26.04.1 LTS".
pub(crate) fn os_version(version: &str) -> String {
    match version.trim().rsplit_once(" (") {
        Some((head, tail)) if tail.ends_with(')') => head.trim().to_owned(),
        _ => version.trim().to_owned(),
    }
}

/// One update row (design §3.3): grouped by kind, phased and held packages excluded.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    pub id: &'static str,
    pub version: Option<String>,
    pub count: usize,
    pub size: Option<u64>,
    pub security: bool,
    pub ids: Vec<String>,
}
pub(crate) fn group(pkgs: &[Pkg], os_version: &str) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    for kind in ROW_ORDER {
        let members: Vec<&Pkg> = pkgs
            .iter()
            .filter(|p| p.offered())
            .filter(|p| match row_kind(&p.name) {
                Some(k) => k == kind,
                None => kind == if p.security() { "security" } else { "other" },
            })
            .collect();
        if members.is_empty() {
            continue;
        }
        let version = match kind {
            "security" => Some(os_version.to_owned()).filter(|v| !v.is_empty()),
            "other" => None,
            _ => members
                .iter()
                .find(|p| primary(kind).iter().any(|n| p.name == *n || (n.ends_with('-') && p.name.starts_with(n))))
                .or(members.first())
                .map(|p| upstream(&p.to)),
        };
        let size = members.iter().map(|p| p.size).sum::<Option<u64>>();
        rows.push(Row {
            id: kind,
            version,
            count: members.len(),
            size,
            security: members.iter().any(|p| p.security()),
            ids: members.iter().map(|p| p.id.clone()).collect(),
        });
    }
    rows
}
/// A row as `job.json` and the snapshot's `lastJob` keep it (the rows an install chose, with their ids).
pub(crate) fn row_info(row: &Row) -> Value {
    json!({"id":row.id,"version":row.version,"count":row.count,"size":row.size,"security":row.security,"ids":row.ids})
}
/// An outcome that makes the job's remaining rows `failed` (a cancel, a declined confirm or an unknown outcome
/// does not).
fn failed_outcome(outcome: &Value) -> Option<Value> {
    let code = outcome["code"].as_str()?;
    (!matches!(code, "cancelled" | "declined" | "unknown_outcome")).then(|| {
        json!({"code":code,"key":outcome["key"],"params":outcome["params"].as_object().cloned().unwrap_or_default()})
    })
}
/// The rows of `update.status` (design §3.3 row states, §4.6 "After Finished"; review F3), pure:
/// * while our job runs (`live`): its per-row states, `pendingDownload` for rows outside it;
/// * after a job, until the next check (`last_job`, the snapshot's record): a row that GetUpdates still lists is
///   `failed` (with the outcome's error) after a failed job, else `pendingDownload` ("success while rows
///   remain"; a row of that kind with only new packages is `pendingDownload` too); a row of the job that is gone
///   is `installed`, or `restart` when one of its packages waits for a reboot (`restart_names`:
///   `/run/reboot-required.pkgs` plus RequireRestart of this boot);
/// * otherwise every row is `pendingDownload`.
///
/// Rows keep the §3.3 order. `error` is a record (`code`, `key`, `params`); the caller renders it.
pub(crate) fn row_views(current: &[Row], live: Option<&Value>, last_job: Option<&Value>, restart_names: &[String]) -> Vec<Value> {
    let last_rows: Vec<&Value> = match (live, last_job) {
        (None, Some(job)) => job["rows"].as_array().map(|a| a.iter().collect()).unwrap_or_default(),
        _ => Vec::new(),
    };
    let failed = last_job.filter(|_| live.is_none()).and_then(|job| failed_outcome(&job["outcome"]));
    let mut out = Vec::new();
    for kind in ROW_ORDER {
        let last = last_rows.iter().find(|r| r["id"] == kind);
        if let Some(row) = current.iter().find(|r| r.id == kind) {
            let (state, pct, error) = match (live, last, &failed) {
                (Some(live), _, _) => {
                    let s = &live["rows"][kind];
                    (s["state"].as_str().unwrap_or("pendingDownload").to_owned(), s["pct"].as_u64().unwrap_or(0), Value::Null)
                }
                // only a row that still holds packages of the failed job (not a new update of that kind)
                (None, Some(last), Some(error)) if last["ids"].as_array().is_some_and(|ids| ids.iter().any(|id| row.ids.iter().any(|r| id == r))) => {
                    ("failed".to_owned(), 0, error.clone())
                }
                _ => ("pendingDownload".to_owned(), 0, Value::Null),
            };
            out.push(json!({"id":row.id,"kind":row.id,"version":row.version,"count":row.count,"size":row.size,
                            "security":row.security,"state":state,"pct":pct,"error":error}));
        } else if let Some(last) = last {
            let restart = last["ids"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|id| id.as_str()?.split(';').next())
                .any(|name| restart_names.iter().any(|n| n == name));
            out.push(json!({"id":kind,"kind":kind,"version":last["version"],"count":last["count"],"size":last["size"],
                            "security":last["security"] == true,"state":if restart { "restart" } else { "installed" },
                            "pct":100,"error":null}));
        }
    }
    out
}

/// `totals` of the snapshot (design §4.3).
pub(crate) fn totals(pkgs: &[Pkg], download_size: Option<u64>) -> Value {
    let offered: Vec<&Pkg> = pkgs.iter().filter(|p| p.offered()).collect();
    let size = download_size.or_else(|| offered.iter().map(|p| p.size).sum::<Option<u64>>());
    json!({"count":offered.len(),"security":offered.iter().filter(|p|p.security()).count(),
           "phased":pkgs.iter().filter(|p|p.phased && !p.held).count(),"held":pkgs.iter().filter(|p|p.held).count(),
           "downloadSize":size})
}

/// The package ids an `update.install {rows?}` may send: only ids of offered packages in the current snapshot,
/// never phased or held ones. None = a row that is not in the snapshot (`update.err.stale`).
pub(crate) fn install_ids(rows: &[Row], wanted: Option<&[String]>) -> Option<Vec<String>> {
    let chosen: Vec<&Row> = match wanted {
        None => rows.iter().collect(),
        Some(ids) => {
            let mut out = Vec::new();
            for id in ids {
                out.push(rows.iter().find(|r| r.id == id)?);
            }
            out
        }
    };
    let ids: Vec<String> = chosen.iter().flat_map(|r| r.ids.iter().cloned()).collect();
    (!ids.is_empty()).then_some(ids)
}

/// A mapped failure: the API error code and the catalogue key (design §3.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Failure {
    pub code: &'static str,
    pub key: &'static str,
}
/// PackageKit's error code (if any) and exit code → our failure. None = success (incl. error 27).
pub(crate) fn map_failure(error: Option<u32>, exit_code: u32) -> Option<Failure> {
    let f = |code, key| Some(Failure { code, key });
    if error == Some(ERROR_NO_PACKAGES_TO_UPDATE) || (error.is_none() && exit_code == exit::SUCCESS) {
        return None;
    }
    match error {
        Some(2 | 37 | 43) => f("network", "update.err.network"),
        Some(66) => f("repair_required", "update.err.repair"),
        Some(25) => f("cannot_cancel", "update.err.cannotCancel"),
        Some(48) => f("not_authorized", "update.err.notAuthorized"),
        Some(46) => f("no_space", "update.err.noSpace"),
        Some(10) => f("download_failed", "update.err.download"),
        Some(26 | 65 | 67) => f("locked", "update.err.locked"),
        Some(5 | 30 | 31 | 51) => f("untrusted", "update.err.untrusted"),
        Some(13 | 35 | 36 | 39) => f("conflict", "update.err.conflict"),
        Some(56 | 58 | 60) => f("dpkg", "update.err.dpkg"),
        Some(17) => f("cancelled", "update.err.cancelled"),
        Some(61) => f("stale", "update.err.stale"),
        Some(_) => match exit_code {
            exit::REPAIR_REQUIRED => f("repair_required", "update.err.repair"),
            _ => f("failed", "update.err.failed"),
        },
        None => match exit_code {
            exit::CANCELLED => f("cancelled", "update.err.cancelled"),
            exit::NEED_UNTRUSTED => f("untrusted", "update.err.untrusted"),
            exit::CANCELLED_PRIORITY => f("locked", "update.err.locked"),
            exit::REPAIR_REQUIRED => f("repair_required", "update.err.repair"),
            _ => f("failed", "update.err.failed"),
        },
    }
}
/// A refresh error for one source: a source warning when the source is not Ubuntu's and GetUpdates worked.
pub(crate) fn source_host(detail: &str) -> Option<String> {
    let start = detail.find("://")? + 3;
    let host: String = detail[start..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-')
        .take(253)
        .collect();
    let ubuntu = host == "ubuntu.com" || host.ends_with(".ubuntu.com") || host.ends_with(".canonical.com");
    (!host.is_empty() && !ubuntu).then_some(host)
}

/// Where one progress value goes (design §4.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Delivery {
    None,
    /// the owner's own page only (`emit_local`)
    Local,
    /// the datagram fan-out, which also reaches the owner's own page (`emit`)
    Both,
}
/// Progress coalescing (design §4.4; review F8): every delivery reaches the owner's own page, so every delivery
/// keeps the 250 ms page interval (≤ 4 per second in any second), and at most one per second also goes to the
/// fan-out. A value that arrives inside an interval is kept and sent by a later timer tick, so the last value is
/// never lost on either path.
#[derive(Clone, Debug, Default)]
pub(crate) struct Progress {
    last_local: Option<Instant>,
    last_remote: Option<Instant>,
    pending_local: bool,
    pending_remote: bool,
}
impl Progress {
    /// A new value at `now`.
    pub(crate) fn offer(&mut self, now: Instant) -> Delivery {
        self.pending_local = true;
        self.pending_remote = true;
        self.flush(now)
    }
    /// A timer tick at `now` (every [`LOCAL_PROGRESS`]).
    pub(crate) fn tick(&mut self, now: Instant) -> Delivery {
        self.flush(now)
    }
    fn flush(&mut self, now: Instant) -> Delivery {
        let due = |last: Option<Instant>, interval: Duration| last.is_none_or(|last| now.duration_since(last) >= interval);
        if !(self.pending_local || self.pending_remote) || !due(self.last_local, LOCAL_PROGRESS) {
            return Delivery::None;
        }
        if self.pending_remote && due(self.last_remote, REMOTE_PROGRESS) {
            self.last_local = Some(now);
            self.last_remote = Some(now);
            self.pending_local = false;
            self.pending_remote = false;
            Delivery::Both
        } else if self.pending_local {
            self.last_local = Some(now);
            self.pending_local = false;
            Delivery::Local
        } else {
            Delivery::None
        }
    }
}
/// The two progress rates: the owner's own page (`emit_local`, 4/s) and the datagram fan-out (`emit`, 1/s).
pub(crate) const LOCAL_PROGRESS: Duration = Duration::from_millis(250);
pub(crate) const REMOTE_PROGRESS: Duration = Duration::from_secs(1);

/// The `update.details` cache (review F15): at most `cap` ids, least recently used first out; cleared on each
/// new GetUpdates.
#[derive(Debug)]
pub(crate) struct Lru {
    cap: usize,
    map: HashMap<String, Value>,
    order: VecDeque<String>,
}
impl Lru {
    pub(crate) fn new(cap: usize) -> Self {
        Self { cap: cap.max(1), map: HashMap::new(), order: VecDeque::new() }
    }
    fn touch(&mut self, id: &str) {
        if let Some(at) = self.order.iter().position(|o| o == id) {
            let key = self.order.remove(at).unwrap_or_default();
            self.order.push_back(key);
        }
    }
    pub(crate) fn get(&mut self, id: &str) -> Option<Value> {
        let value = self.map.get(id).cloned()?;
        self.touch(id);
        Some(value)
    }
    pub(crate) fn contains(&self, id: &str) -> bool {
        self.map.contains_key(id)
    }
    pub(crate) fn insert(&mut self, id: String, value: Value) {
        if self.map.insert(id.clone(), value).is_some() {
            self.touch(&id);
        } else {
            self.order.push_back(id);
        }
        while self.order.len() > self.cap {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            }
        }
    }
    pub(crate) fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }
}

/// What to do with `job.json` when the owner starts (design §4.7, pure).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Reattach {
    /// no job, or a finished one, or one from another boot
    Nothing,
    /// subscribe to this transaction path if `GetTransactionList` still lists it
    Transaction(String),
    /// the job is unfinished in this boot but has no transaction yet: resolve through GetOldTransactions
    OldTransactions,
    /// the job waited for a confirm: end it as declined (nothing was installed)
    Expired,
}
pub(crate) fn reattach(job: Option<&Value>, boot_id: Option<&str>) -> Reattach {
    let Some(job) = job else { return Reattach::Nothing };
    if job.get("finishedAt").is_some_and(|v| !v.is_null()) {
        return Reattach::Nothing;
    }
    if boot_id.is_none() || job["bootId"].as_str() != boot_id {
        return Reattach::Nothing;
    }
    // paused for a confirm when its owner went away (the keep-alive is not held then, review F7): only the
    // simulate ran, so the job ends as declined; GetOldTransactions would report the simulate as a success
    if job["confirm"].is_object() {
        return Reattach::Expired;
    }
    match job["tid"].as_str().filter(|t| t.starts_with('/') && t.len() <= 512) {
        Some(tid) => Reattach::Transaction(tid.to_owned()),
        None => Reattach::OldTransactions,
    }
}
/// A transaction from `TransactionListChanged` that blocks our install: an install/update/remove that is not
/// the transaction of our `job.json` (design §4.7).
pub(crate) fn foreign(role: u32, tid: &str, ours: Option<&str>) -> bool {
    [role::UPDATE_PACKAGES, role::INSTALL_PACKAGES, role::REMOVE_PACKAGES].contains(&role) && Some(tid) != ours
}

/// The release card after a start (design §6.4 step 5), pure. `status_rc` is the wrapper's status file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UpgradeOutcome {
    /// the upgrader runs
    Running,
    /// started less than 120 s ago, not seen yet, no status file: keep waiting
    Starting,
    /// the upgrader ended and a reboot is pending: card hidden, hero restartRequired
    Restart,
    /// never seen, the wrapper reported rc ≠ 0 (126/127/other)
    StartFailed(i64),
    /// never seen, no status file 120 s after the start
    NoWindow,
    /// seen, gone, no reboot, and rc ≠ 0 or no status file
    Ended,
    /// rc 0, not running, no reboot pending: back to the offer
    Offer,
}
pub(crate) const UPGRADE_WINDOW_TIMEOUT: Duration = Duration::from_secs(120);
pub(crate) fn upgrade_outcome(
    running: bool,
    seen: bool,
    status_rc: Option<i64>,
    since_start: Duration,
    reboot_pending: bool,
) -> UpgradeOutcome {
    if running {
        return UpgradeOutcome::Running;
    }
    // The reboot rule only follows an upgrader that ran (or reported success): a reboot that was already
    // pending (today's gnome-shell flag) must not hide a dismissed pkexec dialog (reading of §6.4, gw2.md).
    if reboot_pending && (seen || status_rc == Some(0)) {
        return UpgradeOutcome::Restart;
    }
    match (seen, status_rc) {
        (false, Some(rc)) if rc != 0 => UpgradeOutcome::StartFailed(rc),
        (false, None) if since_start >= UPGRADE_WINDOW_TIMEOUT => UpgradeOutcome::NoWindow,
        (false, None) => UpgradeOutcome::Starting,
        (true, None) => UpgradeOutcome::Ended,
        (true, Some(rc)) if rc != 0 => UpgradeOutcome::Ended,
        _ => UpgradeOutcome::Offer,
    }
}
/// The catalogue key of a start failure's rc (pkexec 126/127, anything else: Ubuntu refused).
pub(crate) fn start_failure_key(rc: i64) -> &'static str {
    match rc {
        126 => "update.release.err.noPassword",
        127 => "update.release.err.wrongPassword",
        _ => "update.release.err.refused",
    }
}
