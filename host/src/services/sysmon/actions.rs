use super::{
    ServiceContext,
    common::*,
    process::{self, ProcessState, argv_of, stat_of},
};
use crate::{
    files::{ApiError, ApiResult},
    i18n::{self, Lang},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ffi::OsString,
    fs,
    os::unix::{fs::MetadataExt, process::CommandExt},
    path::Path,
    process::{Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};
#[derive(Default)]
pub(super) struct ActionState {
    files: (f64, BTreeMap<String, String>),
    eff_prev: HashMap<String, String>,
}
pub(super) fn services(actions: &mut ActionState, pm: &ProcessState) -> Value {
    let units = list_units(None);
    if actions.files.1.is_empty() || now() - actions.files.0 >= 120. {
        actions.files = (now(), unit_files())
    }
    let enabled = &actions.files.1;
    let mut pid_of = HashMap::<String, u32>::new();
    for ((pid, _), st) in &pm.statics {
        if let Some(u) = &st.unit {
            if u.name.ends_with(".service") {
                pid_of
                    .entry(u.name.clone())
                    .and_modify(|p| *p = (*p).min(*pid))
                    .or_insert(*pid);
            }
        }
    }
    let mut out = vec![];
    for u in units {
        let name = u["unit"].as_str().unwrap_or("");
        if !name.ends_with(".service") {
            continue;
        }
        let template = if let Some((base, instance)) = name.rsplit_once('@') {
            if !instance.trim_end_matches(".service").contains('.') {
                format!("{base}@.service")
            } else {
                name.into()
            }
        } else {
            name.into()
        };
        let desc = u["description"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| unescape(name.trim_end_matches(".service")));
        let enabled = enabled
            .get(name)
            .filter(|s| !s.is_empty())
            .or_else(|| enabled.get(&template))
            .cloned();
        out.push(json!({"name":name,"description":desc,"state":u["active"].as_str().unwrap_or(""),"sub":u["sub"].as_str().unwrap_or(""),"scope":"user","load":u["load"].as_str().unwrap_or(""),"enabled":enabled,"pid":pid_of.get(name),"critical":critical_unit(name),"displayName":unescape(name.trim_end_matches(".service"))}));
    }
    out.sort_by_key(|v| v["displayName"].as_str().unwrap_or("").to_lowercase());
    json!(out)
}
pub(super) fn service_action(
    name: &Value,
    action: &Value,
    state: &mut ActionState,
    lang: Lang,
) -> ApiResult {
    let mut name = name
        .as_str()
        .ok_or_else(|| {
            ApiError::tr(
                "bad_args",
                lang,
                "sysmon.service.unknown",
                crate::i18n::NO_PARAMS,
            )
        })?
        .to_owned();
    if !name.ends_with(".service") {
        name.push_str(".service")
    }
    let base = name.strip_suffix(".service").unwrap_or("");
    if base.is_empty()
        || base.len() > 250
        || name.starts_with('-')
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b":_.@\\-".contains(&b))
    {
        return Err(ApiError::tr(
            "bad_args",
            lang,
            "sysmon.service.unknown",
            crate::i18n::NO_PARAMS,
        ));
    }
    let action = action
        .as_str()
        .filter(|s| ["start", "stop", "restart"].contains(s))
        .ok_or_else(|| {
            ApiError::tr(
                "bad_args",
                lang,
                "sysmon.service.unknownAction",
                crate::i18n::NO_PARAMS,
            )
        })?;
    if action != "start" && critical_unit(&name) {
        return Err(ApiError::tr(
            "protected",
            lang,
            "sysmon.service.protected",
            crate::i18n::NO_PARAMS,
        ));
    }
    state.files = (0., BTreeMap::new());
    let r = run("systemctl", &["--user", action, "--", &name], 25.);
    if !r.ok {
        return Err(match action {
            "start" => ApiError::tr(
                "failed",
                lang,
                "sysmon.service.startFailed",
                crate::i18n::NO_PARAMS,
            ),
            "stop" => ApiError::tr(
                "failed",
                lang,
                "sysmon.service.stopFailed",
                crate::i18n::NO_PARAMS,
            ),
            _ => ApiError::tr(
                "failed",
                lang,
                "sysmon.service.restartFailed",
                crate::i18n::NO_PARAMS,
            ),
        });
    }
    Ok(json!({"ok":true}))
}
pub(super) fn pid_list(v: &Value, lang: Lang) -> Result<Vec<u32>, ApiError> {
    let raw = if v.is_i64() || v.is_u64() || v.is_string() || v.is_boolean() {
        vec![v.clone()]
    } else if let Some(a) = v.as_array().filter(|a| !a.is_empty() && a.len() <= 2000) {
        a.clone()
    } else {
        return Err(ApiError::tr(
            "bad_args",
            lang,
            "sysmon.process.none",
            crate::i18n::NO_PARAMS,
        ));
    };
    let mut out = vec![];
    for p in raw {
        let n = valint(&p)
            .and_then(|p| u32::try_from(p).ok())
            .filter(|p| *p > 0)
            .ok_or_else(|| {
                ApiError::tr(
                    "bad_args",
                    lang,
                    "sysmon.process.invalid",
                    crate::i18n::NO_PARAMS,
                )
            })?;
        if !out.contains(&n) {
            out.push(n)
        }
    }
    Ok(out)
}
/// Why Win11 may not end, slow down or restart a process ([`check_own`]); its text is rendered when it is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Own {
    Critical,
    Gone,
    OtherUser,
    Kernel,
}
impl Own {
    #[cfg(test)]
    pub(super) const ALL: [Own; 4] = [Own::Critical, Own::Gone, Own::OtherUser, Own::Kernel];
    /// The catalogue key of the reason (sysmon.tsv).
    pub(super) fn key(self) -> &'static str {
        match self {
            Own::Critical => "sysmon.process.critical",
            Own::Gone => "sysmon.process.gone",
            Own::OtherUser => "sysmon.process.otherUser",
            Own::Kernel => "sysmon.process.kernel",
        }
    }
    /// results[].error of endTask / setEfficiency.
    pub(super) fn message(self, lang: Lang) -> String {
        i18n::tr(lang, self.key())
    }
    /// restartPlan's refusal: `not_found` for a process that is gone, `protected` for the others.
    pub(super) fn refusal(self, lang: Lang) -> ApiError {
        let code = if self == Own::Gone {
            "not_found"
        } else {
            "protected"
        };
        ApiError::tr(code, lang, self.key(), i18n::NO_PARAMS)
    }
}
pub(super) fn check_own(pid: u32, pm: &ProcessState) -> Option<Own> {
    if pid == 1 || pid == std::process::id() {
        return Some(Own::Critical);
    }
    let Some(uid) = metadata_uid(format!("/proc/{pid}")) else {
        return Some(Own::Gone);
    };
    if uid != unsafe { libc::getuid() } {
        return Some(Own::OtherUser);
    }
    let (comm, f) = stat_of(pid).unwrap_or_default();
    if pid == 2 || f.get(1).is_some_and(|s| s == "2") {
        return Some(Own::Kernel);
    }
    if pm.critical.contains(&pid) || is_critical_comm(&comm) {
        return Some(Own::Critical);
    }
    let mut p = unsafe { libc::getppid() } as u32;
    for _ in 0..64 {
        if p <= 1 {
            break;
        }
        if p == pid {
            return Some(Own::Critical);
        }
        p = stat_of(p)
            .and_then(|(_, f)| f.get(1).and_then(|s| s.parse::<u32>().ok()))
            .unwrap_or(0);
    }
    None
}
pub(super) fn end_task(pids: &[u32], force: bool, pm: &ProcessState, lang: Lang) -> Value {
    let mut rows = vec![];
    for pid in pids {
        if let Some(e) = check_own(*pid, pm) {
            rows.push(json!({"pid":pid,"ok":false,"error":e.message(lang)}));
            continue;
        }
        let r = unsafe {
            libc::kill(
                *pid as i32,
                if force { libc::SIGKILL } else { libc::SIGTERM },
            )
        };
        let error = if r == 0 {
            None
        } else {
            let e = std::io::Error::last_os_error();
            match e.raw_os_error() {
                Some(libc::ESRCH) => None,
                Some(libc::EPERM) => Some(crate::i18n::tr(lang, "sysmon.endTask.denied").into()),
                _ => Some(crate::i18n::trf(
                    lang,
                    "sysmon.endTask.failed",
                    &serde_json::json!({"error": e.to_string()}),
                )),
            }
        };
        rows.push(json!({"pid":pid,"ok":error.is_none(),"error":error}));
    }
    json!({"results":rows})
}
fn own_high(unit: &str) -> bool {
    [
        "win11shell-shell-",
        "win11shell-apphost-",
        "app-win11shell-taskmgr-",
    ]
    .iter()
    .any(|p| {
        unit.strip_prefix(p)
            .and_then(|s| s.strip_suffix(".scope"))
            .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
    })
}
pub(super) fn efficiency(
    pids: &[u32],
    enabled: bool,
    state: &mut ActionState,
    pm: &mut ProcessState,
    lang: Lang,
) -> Value {
    let mut rows = vec![];
    let mut done = HashMap::<String, (bool, Option<String>)>::new();
    for pid in pids {
        if let Some(e) = check_own(*pid, pm) {
            rows.push(json!({"pid":pid,"ok":false,"error":e.message(lang)}));
            continue;
        }
        let cg = cgroup(*pid);
        let unit = parse_cgroup(&cg);
        if let Some(unit) = unit.filter(|u| {
            cg.contains("/user@")
                && (u.name.ends_with(".scope") || u.name.ends_with(".service"))
                && u.name != "init.scope"
        }) {
            if critical_unit(&unit.name) {
                rows.push(json!({"pid":pid,"ok":false,"method":"cgroup","error":crate::i18n::tr(lang, "sysmon.efficiency.critical")}));
                continue;
            }
            if own_high(&unit.name) {
                rows.push(json!({"pid":pid,"ok":false,"method":"cgroup","error":crate::i18n::tr(lang, "sysmon.efficiency.ownShell")}));
                continue;
            }
            let (ok, error) = done
                .entry(unit.name.clone())
                .or_insert_with(|| {
                    let weight = if enabled {
                        let r = run(
                            "systemctl",
                            &[
                                "--user",
                                "show",
                                "-p",
                                "CPUWeight",
                                "--value",
                                "--",
                                &unit.name,
                            ],
                            4.,
                        );
                        let prev = r.out.trim();
                        if r.ok
                            && !prev.is_empty()
                            && prev.bytes().all(|b| b.is_ascii_digit())
                            && prev != "10"
                        {
                            state
                                .eff_prev
                                .entry(unit.name.clone())
                                .or_insert_with(|| prev.into());
                        }
                        "10".into()
                    } else {
                        state
                            .eff_prev
                            .remove(&unit.name)
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| "100".into())
                    };
                    let weight = format!("CPUWeight={weight}");
                    let r = run(
                        "systemctl",
                        &[
                            "--user",
                            "set-property",
                            "--runtime",
                            "--",
                            &unit.name,
                            &weight,
                        ],
                        8.,
                    );
                    (
                        r.ok,
                        if r.ok {
                            None
                        } else {
                            Some(crate::i18n::tr(lang, "sysmon.efficiency.failed").into())
                        },
                    )
                })
                .clone();
            rows.push(json!({"pid":pid,"ok":ok,"error":error,"method":"cgroup"}));
            continue;
        }
        let mut tasks = names(format!("/proc/{pid}/task"))
            .into_iter()
            .filter_map(|s| s.parse::<u32>().ok())
            .collect::<Vec<_>>();
        if tasks.is_empty() {
            tasks.push(*pid)
        }
        let mut error = None;
        for tid in tasks {
            let r =
                unsafe { libc::setpriority(libc::PRIO_PROCESS, tid, if enabled { 10 } else { 0 }) };
            if r != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EACCES)
                || r != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
            {
                error = Some(crate::i18n::tr(lang, "sysmon.efficiency.noUndo"));
                break;
            }
        }
        rows.push(json!({"pid":pid,"ok":error.is_none(),"error":error,"method":"nice"}));
    }
    pm.weights = (0., HashMap::new());
    json!({"results":rows})
}
#[derive(Clone, Debug)]
pub(super) struct Plan {
    how: String,
    name: Option<String>,
    pids: Vec<u32>,
    desktop: Option<String>,
    unit: Option<String>,
    via: Option<String>,
    tty: bool,
    argv: Vec<String>,
    exe: String,
    cwd: String,
    env: BTreeMap<OsString, OsString>,
    cwd_changed: bool,
}
impl Plan {
    fn new(how: &str, name: Option<String>) -> Self {
        Self {
            how: how.into(),
            name,
            pids: vec![],
            desktop: None,
            unit: None,
            via: None,
            tty: false,
            argv: vec![],
            exe: String::new(),
            cwd: String::new(),
            env: BTreeMap::new(),
            cwd_changed: false,
        }
    }
    pub fn summary(&self, pid: u32) -> Value {
        let mut out = json!({"pid":pid,"how":self.how,"name":self.name});
        for (k, v) in [
            ("desktopId", &self.desktop),
            ("unit", &self.unit),
            ("via", &self.via),
        ] {
            if let Some(v) = v.as_ref().filter(|s| !s.is_empty()) {
                out[k] = json!(v)
            }
        }
        if self.how == "app" {
            out["count"] = json!(self.pids.len())
        }
        if self.how == "exec" {
            out["tty"] = json!(self.tty)
        }
        out
    }
}
fn ancestors(pid: u32, limit: usize) -> Vec<u32> {
    let mut out = vec![];
    let mut p = pid;
    for _ in 0..limit {
        let Some((_, f)) = stat_of(p) else { break };
        let Some(pp) = f.get(1).and_then(|s| s.parse::<u32>().ok()) else {
            break;
        };
        if pp <= 1 {
            break;
        }
        out.push(pp);
        p = pp;
    }
    out
}
/// The own app window `pid` is or runs below, and its pid: a native app's backend (`--http-only`) restarts its
/// window; the portal's file picker is no Explorer (process::own_app).
fn own_window(pid: u32) -> Option<(String, u32)> {
    let chain = std::iter::once(pid)
        .chain(ancestors(pid, 9))
        .collect::<Vec<_>>();
    for (i, p) in chain.iter().take(9).enumerate() {
        let parent = chain.get(i + 1).map(|q| argv_of(*q)).unwrap_or_default();
        if let Some(app) = process::own_app(&argv_of(*p), &parent) {
            return Some((app, *p));
        }
        if *p == std::process::id() {
            break;
        }
    }
    None
}
fn host_part(pid: u32, argv: &[String], host: u32) -> Option<&'static str> {
    let mut chain = vec![];
    let mut p = pid;
    let mut parent = 0;
    for _ in 0..4 {
        let (s, f) = stat_of(p)?;
        parent = f.get(1)?.parse().ok()?;
        chain.push(s);
        if parent == host {
            break;
        }
        p = parent;
    }
    if parent != host {
        return None;
    }
    let joined = argv.join(" ").to_lowercase();
    if chain.len() <= 2
        && [
            "host.indexer",
            "thumbnailer.py",
            "wl-paste",
            "pw-dump",
            "glycin",
            "hyprsunset",
        ]
        .iter()
        .any(|s| joined.contains(s))
    {
        return Some("helper");
    }
    None
}
fn tty_of(pid: u32) -> i64 {
    stat_of(pid)
        .and_then(|(_, f)| f.get(4).and_then(|s| int(s)))
        .unwrap_or(0)
}
fn program_of(pid: u32) -> String {
    fs::read_link(format!("/proc/{pid}/exe"))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| format!("comm:{}", stat_of(pid).map(|s| s.0).unwrap_or_default()))
}
fn app_tree(pid: u32, members: &HashSet<u32>) -> (u32, Vec<u32>) {
    let parent = members
        .iter()
        .map(|p| {
            (
                *p,
                stat_of(*p)
                    .and_then(|(_, f)| f.get(1).and_then(|s| s.parse::<u32>().ok()))
                    .unwrap_or(0),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut root = pid;
    for _ in 0..64 {
        let pp = parent.get(&root).copied().unwrap_or(0);
        if !members.contains(&pp) {
            break;
        }
        root = pp;
    }
    let mut kids = BTreeMap::<u32, Vec<u32>>::new();
    for (p, pp) in &parent {
        kids.entry(*pp).or_default().push(*p);
    }
    let mut stack = vec![root];
    stack.extend(members.iter().copied().filter(|p| {
        *p != root
            && !members.contains(parent.get(p).unwrap_or(&0))
            && program_of(*p) == program_of(root)
    }));
    let mut out = HashSet::new();
    while let Some(p) = stack.pop() {
        if !out.insert(p) {
            continue;
        }
        let tty = tty_of(p);
        for c in kids.get(&p).into_iter().flatten() {
            let ctty = tty_of(*c);
            if ctty != 0 && ctty != tty {
                continue;
            }
            stack.push(*c);
        }
    }
    let mut rest = out.into_iter().filter(|p| *p != root).collect::<Vec<_>>();
    rest.sort_unstable();
    let mut out = vec![root];
    out.extend(rest);
    (root, out)
}
fn exec_plan(
    pid: u32,
    comm: &str,
    f: &[String],
    mut argv: Vec<String>,
    lang: Lang,
) -> Result<Plan, ApiError> {
    let unsupported = |e: ApiError| with_data(e, json!({"pid":pid}));
    let status = read(format!("/proc/{pid}/status"));
    let ids = |key: &str| {
        status.lines().find_map(|l| {
            l.strip_prefix(key).map(|s| {
                s.split_whitespace()
                    .take(4)
                    .filter_map(|s| s.parse::<u32>().ok())
                    .collect::<Vec<_>>()
            })
        })
    };
    let uids = ids("Uid:");
    let gids = ids("Gid:");
    if uids
        .as_ref()
        .is_none_or(|u| u.len() != 4 || u.iter().any(|u| *u != unsafe { libc::getuid() }))
        || gids
            .as_ref()
            .is_none_or(|g| g.len() != 4 || g.iter().collect::<HashSet<_>>().len() != 1)
    {
        return Err(unsupported(ApiError::tr(
            "unsupported",
            lang,
            "sysmon.restart.setid",
            crate::i18n::NO_PARAMS,
        )));
    }
    let mut exe = fs::read_link(format!("/proc/{pid}/exe"))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let bytes = read_bytes(format!("/proc/{pid}/environ"), 1 << 20).ok_or_else(|| {
        unsupported(ApiError::tr(
            "unsupported",
            lang,
            "sysmon.restart.noEnviron",
            crate::i18n::NO_PARAMS,
        ))
    })?;
    use std::os::unix::ffi::OsStringExt;
    let mut env = BTreeMap::new();
    for item in bytes.split(|b| *b == 0) {
        if let Some(i) = item.iter().position(|b| *b == b'=') {
            env.insert(
                OsString::from_vec(item[..i].to_vec()),
                OsString::from_vec(item[i + 1..].to_vec()),
            );
        }
    }
    if fs::read_link(format!("/proc/{pid}/ns/mnt"))
        .ok()
        .zip(fs::read_link("/proc/self/ns/mnt").ok())
        .is_none_or(|(a, b)| a != b)
    {
        return Err(unsupported(ApiError::tr(
            "unsupported",
            lang,
            "sysmon.restart.sandboxed",
            crate::i18n::NO_PARAMS,
        )));
    }
    if exe.ends_with(" (deleted)") || exe.is_empty() || !Path::new(&exe).exists() {
        exe = if Path::new(&argv[0]).is_absolute() && Path::new(&argv[0]).exists() {
            argv[0].clone()
        } else {
            let envpath = env
                .get(&OsString::from("PATH"))
                .and_then(|v| v.to_str())
                .unwrap_or("/bin:/usr/bin");
            which(&argv[0], Some(envpath))
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        if exe.is_empty() {
            return Err(unsupported(ApiError::tr(
                "unsupported",
                lang,
                "sysmon.restart.exeGone",
                crate::i18n::NO_PARAMS,
            )));
        }
    }
    if fs::metadata(&exe).is_ok_and(|m| m.mode() & (libc::S_ISUID | libc::S_ISGID) != 0) {
        return Err(unsupported(ApiError::tr(
            "unsupported",
            lang,
            "sysmon.restart.setidProgram",
            crate::i18n::NO_PARAMS,
        )));
    }
    if argv.len() >= 2
        && !comm.is_empty()
        && basename(&argv[1]).chars().take(15).collect::<String>() == comm
        && Path::new(&argv[1]).is_absolute()
        && executable(Path::new(&argv[1]))
        && read_bytes(&argv[1], 2).as_deref() == Some(b"#!")
    {
        argv.remove(0);
        exe = argv[0].clone();
    }
    let mut cwd = fs::read_link(format!("/proc/{pid}/cwd"))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let cwd_changed = cwd.ends_with(" (deleted)") || !Path::new(&cwd).is_dir();
    if cwd_changed {
        cwd = home().to_string_lossy().into_owned();
    }
    let pp = f.get(1).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
    let sid = f.get(3).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
    let tty = f.get(4).and_then(|s| int(s)).unwrap_or(0);
    if tty == 0 {
        let pst = if pp > 1 { stat_of(pp) } else { None };
        let parent_ok = pp <= 1
            || pp == std::process::id() && sid == pid
            || pst.as_ref().is_some_and(|(comm, _)| {
                comm == "systemd"
                    && metadata_uid(format!("/proc/{pp}")) == Some(unsafe { libc::getuid() })
            });
        if !parent_ok {
            let pname = pst
                .as_ref()
                .map(|p| p.0.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| crate::i18n::tr(lang, "sysmon.restart.otherProgram"));
            return Err(with_data(
                ApiError::tr(
                    "unsupported",
                    lang,
                    "sysmon.restart.parent",
                    &serde_json::json!({"parent": pname}),
                ),
                json!({"pid":pid,"parent":pp}),
            ));
        }
    }
    let mut plan = Plan::new("exec", Some(comm.into()));
    plan.pids = vec![pid];
    plan.argv = argv;
    plan.exe = exe;
    plan.cwd = cwd;
    plan.env = env;
    plan.tty = tty != 0;
    plan.cwd_changed = cwd_changed;
    Ok(plan)
}
pub(super) fn restart_plan(
    pid: u32,
    pm: &mut ProcessState,
    ctx: &Arc<ServiceContext>,
) -> Result<Plan, ApiError> {
    let lang = ctx.lang();
    if pid == std::process::id() {
        return Ok(Plan::new("shell", Some(tr(lang.code(), "host", &[]))));
    }
    if let Some(e) = check_own(pid, pm) {
        // the code follows the reason, not the text (3b898e6 matched "bestaat niet" in the Dutch message)
        let mut refused = e.refusal(lang);
        refused.data = Some(json!({"pid":pid}));
        return Err(refused);
    }
    let (comm, f) = stat_of(pid).ok_or_else(|| {
        with_data(
            ApiError::tr(
                "not_found",
                lang,
                "sysmon.process.gone",
                crate::i18n::NO_PARAMS,
            ),
            json!({"pid":pid}),
        )
    })?;
    let argv = argv_of(pid);
    if argv.is_empty() {
        return Err(with_data(
            ApiError::tr(
                "unsupported",
                lang,
                "sysmon.restart.noCommand",
                crate::i18n::NO_PARAMS,
            ),
            json!({"pid":pid}),
        ));
    }
    if let Some((app, wpid)) = own_window(pid) {
        let (name, _) = process::own_names(lang, &app);
        let mut plan = Plan::new("app", Some(name));
        plan.desktop = Some(format!("win11shell-{app}.desktop"));
        plan.pids = vec![wpid];
        if pid != wpid && !comm.is_empty() {
            plan.via = Some(comm)
        }
        return Ok(plan);
    }
    match host_part(pid, &argv, std::process::id()) {
        Some("helper") => {
            return Err(with_data(
                ApiError::tr(
                    "protected",
                    lang,
                    "sysmon.restart.shellPart",
                    crate::i18n::NO_PARAMS,
                ),
                json!({"pid":pid}),
            ));
        }
        _ => {}
    }
    let cg = cgroup(pid);
    if let Some(u) = parse_cgroup(&cg).filter(|u| {
        cg.contains("/user@") && u.name.ends_with(".service") && !u.name.starts_with("app-")
    }) {
        if critical_unit(&u.name) {
            return Err(with_data(
                ApiError::tr(
                    "protected",
                    lang,
                    "sysmon.restart.criticalService",
                    crate::i18n::NO_PARAMS,
                ),
                json!({"pid":pid,"unit":u.name}),
            ));
        }
        let mut plan = Plan::new(
            "service",
            Some(unit_friendly(lang.code(), &u.name).unwrap_or_else(|| u.name.clone())),
        );
        plan.unit = Some(u.name);
        return Ok(plan);
    }
    let data = pm.processes(ctx, false);
    for g in data["apps"].as_array().into_iter().flatten() {
        let did = g["desktopId"].as_str().filter(|s| s.ends_with(".desktop"));
        let pids = g["pids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p.as_u64().map(|p| p as u32))
            .collect::<Vec<_>>();
        if pids.contains(&pid) {
            if let Some(did) = did {
                let members = pids
                    .into_iter()
                    .filter(|p| check_own(*p, pm).is_none())
                    .collect::<HashSet<_>>();
                if members.is_empty() {
                    continue;
                }
                let (root, tree) = app_tree(pid, &members);
                if !tree.contains(&pid) {
                    return exec_plan(pid, &comm, &f, argv, lang);
                }
                let mut plan = Plan::new(
                    "app",
                    Some(
                        g["name"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .unwrap_or(did)
                            .into(),
                    ),
                );
                plan.desktop = Some(did.into());
                plan.pids = tree;
                if pid != root && !comm.is_empty() {
                    plan.via = Some(comm)
                }
                return Ok(plan);
            }
        }
    }
    exec_plan(pid, &comm, &f, argv, lang)
}
pub(super) fn confirm(plan: &Plan, confirmed: bool, pid: u32, lang: Lang) -> Result<(), ApiError> {
    if plan.how == "exec" && plan.tty && !confirmed {
        return Err(with_data(
            ApiError::tr(
                "confirm_required",
                lang,
                "sysmon.restart.terminal",
                crate::i18n::NO_PARAMS,
            ),
            json!({"pid":pid,"reason":"terminal"}),
        ));
    }
    Ok(())
}
fn alive(pid: u32, start: u64) -> bool {
    stat_of(pid).is_some_and(|(_, f)| {
        f.len() > 19 && f[0] != "Z" && f[0] != "X" && f[19].parse::<u64>().ok() == Some(start)
    })
}
fn end_gracefully(pids: &[u32], term: f64, kill: f64) -> Vec<u32> {
    let starts = pids
        .iter()
        .filter_map(|p| {
            stat_of(*p)
                .and_then(|(_, f)| f.get(19).and_then(|s| s.parse::<u64>().ok()))
                .map(|s| (*p, s))
        })
        .collect::<BTreeMap<_, _>>();
    for (signal, wait) in [(libc::SIGTERM, term), (libc::SIGKILL, kill)] {
        let left = starts
            .iter()
            .filter(|(p, s)| alive(**p, **s))
            .map(|(p, _)| *p)
            .collect::<Vec<_>>();
        if left.is_empty() {
            return vec![];
        }
        for p in &left {
            if unsafe { libc::kill(*p as i32, signal) } != 0
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
            {
                return left;
            }
        }
        let deadline = Instant::now() + Duration::from_secs_f64(wait);
        while Instant::now() < deadline {
            if !starts.iter().any(|(p, s)| alive(*p, *s)) {
                return vec![];
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    starts
        .into_iter()
        .filter(|(p, s)| alive(*p, *s))
        .map(|(p, _)| p)
        .collect()
}
pub(super) fn restart(
    pid: u32,
    plan: Plan,
    confirmed: bool,
    ctx: &Arc<ServiceContext>,
    actions: &mut ActionState,
) -> ApiResult {
    let name = plan.name.clone();
    let lang = ctx.lang();
    if plan.how == "shell" {
        ctx.call("shell.restart", &json!({}))?;
        return Ok(json!({"ok":true,"pid":pid,"how":"shell","name":name}));
    }
    if plan.how == "service" {
        let unit = plan.unit.as_ref().unwrap();
        service_action(&json!(unit), &json!("restart"), actions, lang)?;
        return Ok(json!({"ok":true,"pid":pid,"how":"service","unit":unit,"name":name}));
    }
    confirm(&plan, confirmed, pid, lang)?;
    let left = end_gracefully(&plan.pids, 5., 2.);
    if !left.is_empty() {
        return Err(with_data(
            ApiError::tr(
                "failed",
                lang,
                "sysmon.restart.notStopped",
                crate::i18n::NO_PARAMS,
            ),
            json!({"pids":left}),
        ));
    }
    if plan.how == "app" {
        let did = plan.desktop.as_ref().unwrap();
        if let Err(e) = ctx.call("apps.launch", &json!({"id":did})) {
            return Err(with_data(
                ApiError::tr(
                    "launch_failed",
                    lang,
                    "sysmon.restart.reopenFailed",
                    &serde_json::json!({"message": e.message.to_string()}),
                ),
                json!({"desktopId":did}),
            ));
        }
        return Ok(
            json!({"ok":true,"pid":pid,"how":"app","desktopId":did,"ended":plan.pids,"name":name}),
        );
    }
    let mut cmd = Command::new(&plan.exe);
    cmd.arg0(&plan.argv[0])
        .args(&plan.argv[1..])
        .current_dir(&plan.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let env = crate::files::actions::restart_environment(plan.env);
    let new_pid = crate::process_scope::spawn_pid_with_argv0(
        cmd,
        "sysmon-restart",
        Some(env.into_iter().collect()),
        Some(plan.argv[0].clone().into()),
    )
    .map_err(|e| {
        with_data(
            ApiError::tr(
                "launch_failed",
                lang,
                "sysmon.restart.respawnFailed",
                &serde_json::json!({"error": e.to_string()}),
            ),
            json!({"pid":pid}),
        )
    })?;
    Ok(
        json!({"ok":true,"pid":pid,"how":"exec","newPid":new_pid,"name":name,"cwdChanged":plan.cwd_changed}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pid_inputs_preserve_order_and_reject_bad_values() {
        assert_eq!(pid_list(&json!(["2", 3, 2]), Lang::Nl).unwrap(), vec![2, 3]);
        for value in [json!([]), json!(null), json!({})] {
            assert_eq!(
                pid_list(&value, Lang::Nl).unwrap_err().message,
                "Geen processen opgegeven."
            );
        }
        for value in [json!([0]), json!(["x"]), json!([-1])] {
            assert_eq!(
                pid_list(&value, Lang::Nl).unwrap_err().message,
                "Ongeldig proces."
            );
        }
    }
    #[test]
    fn protected_processes_never_reach_signal() {
        let pm = ProcessState::default();
        assert_eq!(check_own(1, &pm), Some(Own::Critical));
        assert_eq!(check_own(std::process::id(), &pm), Some(Own::Critical));
        assert!(Own::Critical.message(Lang::Nl).contains("bureaublad"));
    }
    #[test]
    fn terminal_confirmation_carries_original_dialog_data() {
        let mut p = Plan::new("exec", Some("test".into()));
        p.tty = true;
        let e = confirm(&p, false, 42, Lang::Nl).unwrap_err();
        assert_eq!(e.code, "confirm_required");
        assert_eq!(e.data, json!({"pid":42,"reason":"terminal"}).into());
        assert!(confirm(&p, true, 42, Lang::Nl).is_ok());
    }
    // A native app runs as its window `win11-gtk-<kind>` with the backend `win11-app --app <kind> --http-only` below
    // it (native/kit backend.rs, PR_SET_PDEATHSIG): restarting the backend or its child restarts the window (the
    // backend follows it), and an Explorer that win11-file-portal started (the file picker) is never restarted as
    // Explorer. Only plans are made here; nothing is ended.
    #[test]
    fn a_native_backend_restarts_its_window_and_the_picker_is_no_explorer() {
        use super::process::tree::Tree;
        let dir = "/nonexistent/winpoint-test";
        let app = Tree::spawn(
            &format!("{dir}/win11-gtk-explorer"),
            &format!(
                "(exec -a {dir}/win11-app bash -c 'sleep 30; :' x --app explorer --http-only) & wait"
            ),
        );
        let backend = app.child(app.root(), &format!("{dir}/win11-app"));
        let worker = app.child(backend, "sleep");
        let portal = Tree::spawn(
            &format!("{dir}/win11-file-portal"),
            &format!("(exec -a {dir}/win11-gtk-explorer sleep 30) & wait"),
        );
        let picker = portal.child(portal.root(), &format!("{dir}/win11-gtk-explorer"));
        for pid in [app.root(), backend, worker] {
            assert_eq!(
                own_window(pid),
                Some(("explorer".into(), app.root())),
                "{pid}"
            );
        }
        assert_eq!(own_window(picker), None);
        assert_eq!(own_window(portal.root()), None);
        let root =
            std::env::temp_dir().join(format!("win11-sysmon-restart-{}", crate::events::nonce()));
        let config = root.join("config");
        fs::create_dir_all(&config).unwrap();
        fs::write(
            config.join("settings.json"),
            json!({"language":"en"}).to_string(),
        )
        .unwrap();
        let ctx = ServiceContext::new(
            crate::events::Events::new(&root.join("events")).unwrap(),
            config,
            root.join("cache"),
            root.join("ui"),
            "taskmgr".into(),
            false,
        )
        .unwrap();
        let mut pm = ProcessState::default();
        let plan = restart_plan(backend, &mut pm, &ctx).unwrap();
        assert_eq!(
            (
                plan.how.as_str(),
                plan.name.as_deref(),
                plan.desktop.as_deref()
            ),
            (
                "app",
                Some("File Explorer"),
                Some("win11shell-explorer.desktop")
            )
        );
        assert_eq!(plan.via.as_deref(), Some("bash"));
        assert_eq!(plan.pids, vec![app.root()]);
        ctx.events.shutdown();
        let _ = fs::remove_dir_all(root);
    }
}
