//! Actions on processes, shared by Processen and Details (js/apps/taskmgr/actions.js): end task (with a friendly
//! confirmation and a "force" second step), restart (the host says first what it would do, sysmon.restartPlan,
//! and Task Manager asks the matching question: an app closes with all its windows, always asked; one process or
//! a service after a confirmation; our own shell: "Win11 Shell opnieuw starten", like Windows' "Windows
//! Verkenner opnieuw starten"), efficiency mode, open file location, search online, properties and the context
//! menu. Priority and affinity are not offered: the backend has no method for them (api-methods.json).
use crate::app::App;
use crate::hostcall::{self, Opts, State};
use crate::table::TmTable;
use crate::util::{self, Icon, TmGlyph, fill, plural};
use kit::controls::{Dialog, DialogResult, MenuItem, Toast, TopAction};
use kit::prelude::*;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

/// The texts of actions.js.
pub fn tr(key: &str) -> &'static str {
    let (nl, en) = match key {
        "endTask" => ("Taak beëindigen", "End task"),
        "forceEnd" => ("Geforceerd beëindigen", "Force end"),
        "cancel" => ("Annuleren", "Cancel"),
        "endTitle" => ("\u{2018}{name}\u{2019} beëindigen?", "End \u{2018}{name}\u{2019}?"),
        "endBody" => (
            "{name} wordt meteen gesloten. Werk dat je nog niet hebt opgeslagen, kan verloren gaan.",
            "{name} will close right away. Any work you haven\u{2019}t saved might be lost.",
        ),
        "endBodyManyOne" => (
            "Dit sluit {n} proces van {name}. Werk dat je nog niet hebt opgeslagen, kan verloren gaan.",
            "{n} process of {name} will close. Any work you haven\u{2019}t saved might be lost.",
        ),
        "endBodyManyOther" => (
            "Dit sluit {n} processen van {name}. Werk dat je nog niet hebt opgeslagen, kan verloren gaan.",
            "All {n} processes of {name} will close. Any work you haven\u{2019}t saved might be lost.",
        ),
        "endTip" => ("Tip: lukt het nog om de app zelf te sluiten? Dan is dat veiliger.", "Tip: if you can still close the app yourself, that\u{2019}s safer."),
        "endTitleMultiOne" => ("{n} proces beëindigen?", "End {n} process?"),
        "endTitleMultiOther" => ("{n} processen beëindigen?", "End {n} processes?"),
        "endBodyMulti" => (
            "Deze processen worden meteen gesloten: {list}. Werk dat je nog niet hebt opgeslagen, kan verloren gaan.",
            "These processes will close right away: {list}. Any work you haven\u{2019}t saved might be lost.",
        ),
        "dontAsk" => ("Niet meer vragen", "Don\u{2019}t ask again"),
        "hangTitle" => ("\u{2018}{name}\u{2019} reageert niet", "\u{2018}{name}\u{2019} is not responding"),
        "hangBody" => (
            "{name} heeft het verzoek om te stoppen genegeerd. Je kunt de app geforceerd beëindigen; werk dat niet is opgeslagen, gaat dan verloren.",
            "{name} ignored the request to stop. You can force it to end; unsaved work will then be lost.",
        ),
        "ended" => ("{name} is beëindigd", "{name} has ended"),
        "endFailed" => ("{name} kon niet worden beëindigd", "{name} couldn\u{2019}t be ended"),
        "dryRun" => ("Testmodus: er is niets echt beëindigd", "Test mode: nothing was actually ended"),
        "systemProc" => ("Systeemproces \u{2013} kan niet worden beëindigd", "System process \u{2013} can\u{2019}t be ended"),
        "effTitle" => ("Efficiëntiemodus inschakelen voor \u{2018}{name}\u{2019}?", "Turn on efficiency mode for \u{2018}{name}\u{2019}?"),
        "effBody" => (
            "{name} krijgt minder voorrang van de processor. Andere apps blijven daardoor vlot en je pc verbruikt minder stroom. De app zelf kan wat trager reageren.",
            "{name} gets a lower priority on the processor. Other apps stay responsive and your PC uses less power. The app itself may respond a little slower.",
        ),
        "effOn" => ("Efficiëntiemodus inschakelen", "Turn on efficiency mode"),
        "efficiency" => ("Efficiëntiemodus", "Efficiency mode"),
        "effEnabled" => ("Efficiëntiemodus staat aan voor {name}", "Efficiency mode is on for {name}"),
        "effDisabled" => ("Efficiëntiemodus staat uit voor {name}", "Efficiency mode is off for {name}"),
        "effFailed" => ("Efficiëntiemodus kon niet worden aangepast", "Efficiency mode couldn\u{2019}t be changed"),
        "details" => ("Details bekijken", "Go to details"),
        "fileLocation" => ("Bestandslocatie openen", "Open file location"),
        "searchOnline" => ("Online zoeken", "Search online"),
        "properties" => ("Eigenschappen", "Properties"),
        "expand" => ("Uitvouwen", "Expand"),
        "collapse" => ("Samenvouwen", "Collapse"),
        "noLocation" => ("Locatie onbekend", "Location unknown"),
        "propsTitle" => ("Eigenschappen van {name}", "{name} Properties"),
        "close" => ("Sluiten", "Close"),
        "pName" => ("Naam", "Name"),
        "pDesc" => ("Omschrijving", "Description"),
        "pPid" => ("Proces-id (PID)", "Process ID (PID)"),
        "pPids" => ("Processen", "Processes"),
        "pUser" => ("Gebruiker", "User"),
        "pStatus" => ("Status", "Status"),
        "pStarted" => ("Gestart", "Started"),
        "pCpu" => ("Processor", "CPU"),
        "pMem" => ("Geheugen", "Memory"),
        "pDisk" => ("Schijf", "Disk"),
        "pThreads" => ("Threads", "Threads"),
        "pCmd" => ("Opdrachtregel", "Command line"),
        "pLoc" => ("Locatie", "Location"),
        "pWindows" => ("Vensters", "Windows"),
        "pCountOne" => ("{n} proces", "{n} process"),
        "pCountOther" => ("{n} processen", "{n} processes"),
        "copy" => ("Kopiëren", "Copy"),
        "copied" => ("Gekopieerd", "Copied"),
        "st_running" => ("Actief", "Running"),
        "st_notResponding" => ("Reageert niet", "Not responding"),
        "st_suspended" => ("Onderbroken", "Suspended"),
        "st_efficiency" => ("Efficiëntiemodus", "Efficiency mode"),
        "failedGeneric" => ("Er ging iets mis. Probeer het opnieuw.", "Something went wrong. Try again."),
        "restart" => ("Opnieuw starten", "Restart"),
        "restartShell" => ("Win11 Shell opnieuw starten", "Restart Win11 Shell"),
        "restartTitle" => ("\u{2018}{name}\u{2019} opnieuw starten?", "Restart \u{2018}{name}\u{2019}?"),
        "restartBody" => (
            "{name} wordt gestopt en meteen weer gestart. Dat helpt vaak als iets is blijven hangen.",
            "{name} will stop and start again right away. This often helps when something is stuck.",
        ),
        "restartBodyService" => (
            "De service {name} wordt gestopt en meteen weer gestart. Dat helpt vaak als iets is blijven hangen.",
            "The service {name} will stop and start again right away. This often helps when something is stuck.",
        ),
        "restartHint" => ("Werk dat nog niet is opgeslagen, kan verloren gaan.", "Any work you haven\u{2019}t saved might be lost."),
        "restartBodyApp" => (
            "\u{2018}{proc}\u{2019} hoort bij {name}. Daarom wordt {name} helemaal gesloten en meteen weer geopend, met al zijn vensters.",
            "\u{2018}{proc}\u{2019} belongs to {name}, so {name} will close completely, with all its windows, and open again right away.",
        ),
        "restartBodyAppRow" => (
            "{name} wordt helemaal gesloten en meteen weer geopend, met al zijn vensters.",
            "{name} will close completely, with all its windows, and open again right away.",
        ),
        "restarting" => ("{name} wordt opnieuw gestart\u{2026}", "Restarting {name}\u{2026}"),
        "restarted" => ("{name} is opnieuw gestart", "{name} has restarted"),
        "restartFailed" => ("{name} kon niet opnieuw worden gestart", "{name} couldn\u{2019}t be restarted"),
        "restartDryRun" => ("Testmodus: er is niets echt opnieuw gestart", "Test mode: nothing was actually restarted"),
        "dry_exec" => ("\u{2018}{name}\u{2019} zou stoppen en met dezelfde opdracht opnieuw starten.", "\u{2018}{name}\u{2019} would stop and start again with the same command."),
        "dry_app" => ("{name} zou sluiten en opnieuw openen.", "{name} would close and open again."),
        "dry_service" => ("De service {name} zou opnieuw starten.", "The service {name} would restart."),
        "dry_shell" => ("Win11 Shell zou opnieuw starten.", "Win11 Shell would restart."),
        "restartSystem" => ("Systeemproces \u{2013} kan niet opnieuw worden gestart", "System process \u{2013} can\u{2019}t be restarted"),
        "restartPickOne" => ("Vouw de groep uit en kies één proces om opnieuw te starten", "Expand the group and choose one process to restart"),
        "restartOne" => ("Selecteer één proces om opnieuw te starten", "Select one process to restart"),
        "restartUnsupported" => ("Deze versie van Win11 Shell kan nog geen processen opnieuw starten.", "This version of Win11 Shell can\u{2019}t restart processes yet."),
        "restartAnyway" => ("Toch opnieuw starten", "Restart anyway"),
        "shellFailed" => ("Win11 Shell kon niet opnieuw worden gestart", "Win11 Shell couldn\u{2019}t be restarted"),
        "shellDryRun" => ("Testmodus: Win11 Shell is niet echt opnieuw gestart", "Test mode: Win11 Shell wasn\u{2019}t actually restarted"),
        "shellTip" => ("De taakbalk en het bureaublad starten opnieuw.", "The taskbar and the desktop will restart."),
        _ => ("", ""),
    };
    kit::t(nl, en)
}

// ------------------------------------------------------------------------------------------------ rows

/// A process row as Processen and Details build it (an app or background group, a system group, one process,
/// or several selected processes together).
#[derive(Clone, Debug, Default)]
pub struct ProcRow {
    /// "app", "bg", "sys", "proc" or "multi".
    pub kind: &'static str,
    pub key: String,
    pub name: String,
    pub display_name: String,
    /// An app icon name ("" for none).
    pub icon: String,
    pub pids: Vec<i64>,
    pub procs: Rc<Vec<Value>>,
    pub cpu: f64,
    pub memory: f64,
    pub disk: f64,
    pub threads: f64,
    /// running, notResponding, suspended or efficiency.
    pub status: &'static str,
    pub windows: Vec<String>,
    pub can_kill: bool,
    pub efficiency: bool,
    pub cmd: String,
    pub exe: String,
    pub user: String,
    pub proc_name: String,
    pub pid: Option<i64>,
    /// Details: the process's group ("app", "background", "system").
    pub group: String,
    pub state_text: String,
    pub multi: bool,
    pub children: Vec<Rc<ProcRow>>,
    /// Processes: the group row of a child process (a copy without its children).
    pub parent: Option<Rc<ProcRow>>,
    /// Details: the app a process belongs to.
    pub app_row: Option<Rc<ProcRow>>,
}

/// The host's status code of an app (running | notResponding | suspended | efficiency); other spellings by their
/// English words, never by display text.
pub fn norm_status(s: &str) -> &'static str {
    match s.to_lowercase().as_str() {
        "running" => return "running",
        "notresponding" => return "notResponding",
        "suspended" => return "suspended",
        "efficiency" => return "efficiency",
        _ => {}
    }
    let v = s.to_lowercase().replace([' ', '_'], "-");
    if v.contains("not-responding") || v.contains("notresponding") || v.contains("unresponsive") || v.contains("hung") {
        "notResponding"
    } else if v.contains("suspend") || v.contains("stopped") {
        "suspended"
    } else if v.contains("efficien") {
        "efficiency"
    } else {
        "running"
    }
}

pub fn pid_of(p: &Value) -> Option<i64> {
    p["pid"].as_i64()
}

/// The action target of a row (actions.js targetOf).
pub struct Target {
    pub name: String,
    pub icon: String,
    pub pids: Vec<i64>,
    pub procs: Rc<Vec<Value>>,
    pub can_kill: bool,
    pub efficiency: bool,
    pub cmd: String,
    pub exe: String,
}

pub fn target_of(row: &ProcRow) -> Target {
    let procs = row.procs.clone();
    let pids = if row.pids.is_empty() { procs.iter().filter_map(pid_of).collect() } else { row.pids.clone() };
    Target {
        name: row.name.clone(),
        icon: row.icon.clone(),
        pids,
        can_kill: if procs.is_empty() { row.can_kill } else { procs.iter().all(|p| p["canKill"] != false) },
        efficiency: if procs.is_empty() { row.efficiency } else { procs.iter().all(|p| p["efficiency"] == true) },
        cmd: if row.cmd.is_empty() { util::text(&procs.first().cloned().unwrap_or_default()["cmd"]) } else { row.cmd.clone() },
        exe: if row.exe.is_empty() { util::text(&procs.first().cloned().unwrap_or_default()["exe"]) } else { row.exe.clone() },
        procs,
    }
}

/// Several selected rows as one action target (Details).
pub fn combine(rows: &[Rc<ProcRow>]) -> Rc<ProcRow> {
    if rows.len() == 1 {
        return rows[0].clone();
    }
    let procs: Vec<Value> = rows.iter().flat_map(|r| r.procs.iter().cloned()).collect();
    Rc::new(ProcRow {
        kind: "multi",
        name: plural(rows.len() as i64, kit::t("{n} proces", "{n} process"), kit::t("{n} processen", "{n} processes")),
        pids: procs.iter().filter_map(pid_of).collect(),
        procs: Rc::new(procs),
        multi: true,
        can_kill: true,
        status: "running",
        ..Default::default()
    })
}

fn list_names(procs: &[Value]) -> String {
    let names: Vec<String> = procs.iter().map(|p| format!("{} ({})", util::text(&p["name"]), util::text(&p["pid"]))).collect();
    if names.len() > 6 { format!("{} \u{2026}", names[..6].join(", ")) } else { names.join(", ") }
}

fn icon_for(tg_icon: &str, size: i32) -> gtk::Widget {
    if !tg_icon.is_empty() {
        return kit::icons::app_icon(tg_icon, size).upcast();
    }
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    b.add_css_class("taskmgr-dlg-glyph");
    b.set_size_request(size, size);
    b.set_valign(gtk::Align::Start);
    b.set_hexpand(false);
    let g = kit::icons::glyph("process", (size as f64 * 0.7).round() as i32);
    g.set_hexpand(true);
    b.append(&g);
    b.upcast()
}

/// The dialog body of actions.js: an icon, paragraphs (the last ones as hints), optionally a check box.
fn dlg_body(icon: gtk::Widget, texts: &[&str], hint: Option<&str>, check: Option<&gtk::CheckButton>) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 16);
    root.add_css_class("taskmgr-dlg");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    icon.set_valign(gtk::Align::Start);
    row.append(&icon);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 8);
    text.set_hexpand(true);
    for t in texts {
        let l = util::wrap_label(t, "w11-dialog-text");
        l.set_max_width_chars(48);
        text.append(&l);
    }
    if let Some(h) = hint {
        let l = util::wrap_label(h, "taskmgr-dlg-hint");
        l.set_max_width_chars(48);
        text.append(&l);
    }
    row.append(&text);
    root.append(&row);
    if let Some(c) = check {
        root.append(c);
    }
    root
}

fn after(ms: u64, f: impl FnOnce() + 'static) {
    glib::timeout_add_local_once(Duration::from_millis(ms), f);
}

fn message_of(e: &kit::CallError) -> String {
    if e.message.is_empty() { tr("failedGeneric").to_owned() } else { e.message.clone() }
}

// ------------------------------------------------------------------------------------------------ end task

/// End the row's processes (asked first while taskmgr.confirmEndTask is on).
pub fn end_task(app: &Rc<App>, row: &ProcRow) {
    end_task_with(app, row, false);
}

/// `end_task` that always asks first, whatever the settings say (the `open=end` test helper: it must never end
/// anything by itself).
pub fn ask_end_task(app: &Rc<App>, row: &ProcRow) {
    end_task_with(app, row, true);
}

fn end_task_with(app: &Rc<App>, row: &ProcRow, always_ask: bool) {
    let tg = target_of(row);
    if !tg.can_kill || tg.pids.is_empty() {
        return;
    }
    if always_ask || app.setting("taskmgr.confirmEndTask").as_bool().unwrap_or(true) {
        let check = kit::controls::checkbox(tr("dontAsk"));
        let n = tg.pids.len() as i64;
        let body_text = if row.multi {
            fill(tr("endBodyMulti"), &[("list", &list_names(&tg.procs))])
        } else if n > 1 {
            fill(&plural(n, tr("endBodyManyOne"), tr("endBodyManyOther")), &[("name", &tg.name)])
        } else {
            fill(tr("endBody"), &[("name", &tg.name)])
        };
        let title = if row.multi { plural(n, tr("endTitleMultiOne"), tr("endTitleMultiOther")) } else { fill(tr("endTitle"), &[("name", &tg.name)]) };
        let body = dlg_body(icon_for(&tg.icon, 32), &[&body_text], Some(tr("endTip")), Some(&check));
        let app2 = app.clone();
        Dialog::new(&title)
            .body(&body)
            .primary(tr("endTask"))
            .close_label(tr("cancel"))
            .danger()
            .width(460)
            .show(&app.parent(), move |r| {
                if r != DialogResult::Primary {
                    return;
                }
                if check.is_active() {
                    kit::ctx().set_setting("taskmgr.confirmEndTask", json!(false));
                }
                do_end(&app2, tg, false);
            });
        return;
    }
    do_end(app, tg, false);
}

fn do_end(app: &Rc<App>, tg: Target, force: bool) {
    let app = app.clone();
    hostcall::call("sysmon.endTask", json!({ "pids": tg.pids, "force": force }), move |r| {
        let res = match r {
            Ok(res) => res,
            Err(e) => {
                app.toast_error(&fill(tr("endFailed"), &[("name", &tg.name)]), &message_of(&e));
                return;
            }
        };
        let results: Vec<Value> = res["results"].as_array().cloned().unwrap_or_default();
        let failed: Vec<&Value> = results.iter().filter(|r| r["ok"] != true).collect();
        if res["dryRun"] == true {
            app.toast_info(tr("dryRun"), "");
            app.store.refresh("processes");
            return;
        }
        if !failed.is_empty() && failed.len() == results.len() {
            let msg = failed[0]["error"].as_str().filter(|s| !s.is_empty()).unwrap_or(tr("failedGeneric")).to_owned();
            let a = app.clone();
            app.alert_error(&fill(tr("endFailed"), &[("name", &tg.name)]), &msg, move || a.store.refresh("processes"));
            return;
        }
        let ended = fill(tr("ended"), &[("name", &tg.name)]);
        if force {
            after(400, move || {
                let a = app.clone();
                app.store.refresh_then("processes", move |_| a.toast_success(&ended, 3500));
            });
            return;
        }
        // give the app a moment to close itself, then check
        after(1800, move || {
            let a = app.clone();
            app.store.refresh_then("processes", move |data| {
                let alive: HashSet<i64> = data.as_ref().and_then(|d| d["processes"].as_array().cloned()).unwrap_or_default().iter().filter_map(pid_of).collect();
                let still: Vec<i64> = tg.pids.iter().copied().filter(|p| alive.contains(p)).collect();
                if still.is_empty() {
                    a.toast_success(&ended, 3500);
                    return;
                }
                let body = dlg_body(icon_for(&tg.icon, 32), &[&fill(tr("hangBody"), &[("name", &tg.name)])], None, None);
                let a2 = a.clone();
                Dialog::new(&fill(tr("hangTitle"), &[("name", &tg.name)]))
                    .body(&body)
                    .primary(tr("forceEnd"))
                    .close_label(tr("cancel"))
                    .danger()
                    .width(460)
                    .show(&a.parent(), move |r| {
                        if r == DialogResult::Primary {
                            do_end(&a2, Target { pids: still, ..tg }, true);
                        }
                    });
            });
        });
    });
}

// ------------------------------------------------------------------------------------------------- restart

const SHELL_KEY: &str = "win11shell";

/// The processes of a group whose parent is not in the group (the "main" processes), lowest pid first.
pub fn roots_of(procs: &[Value]) -> Vec<Value> {
    let pids: HashSet<i64> = procs.iter().filter_map(pid_of).collect();
    let mut out: Vec<Value> = procs.iter().filter(|p| !p["ppid"].as_i64().is_some_and(|pp| pids.contains(&pp))).cloned().collect();
    out.sort_by_key(|p| pid_of(p).unwrap_or(0));
    out
}

/// The host's pid (root of the "Win11 Shell" group) from a sysmon.processes answer.
pub fn shell_pid(data: Option<&Value>) -> Option<i64> {
    let data = data?;
    let all = data["processes"].as_array().cloned().unwrap_or_default();
    let g = data["background"].as_array().and_then(|b| b.iter().find(|g| g["key"] == SHELL_KEY).cloned());
    let procs: Vec<Value> = match &g {
        Some(g) => {
            let pids: Vec<i64> = g["pids"].as_array().map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default();
            all.iter().filter(|p| pid_of(p).is_some_and(|pid| pids.contains(&pid))).cloned().collect()
        }
        None => all.iter().filter(|p| p["groupKey"] == SHELL_KEY).cloned().collect(),
    };
    roots_of(&procs).first().and_then(pid_of).or_else(|| g.and_then(|g| g["pids"][0].as_i64()))
}

/// True for the Win11 Shell row, its child processes and the host's processes in Details.
pub fn is_shell_row(row: &ProcRow) -> bool {
    let key = format!("b:{SHELL_KEY}");
    if row.key == key || row.parent.as_ref().is_some_and(|p| p.key == key) {
        return true;
    }
    !row.procs.is_empty() && row.procs.iter().all(|p| p["groupKey"] == SHELL_KEY)
}

/// What "Opnieuw starten" does for a row, as far as the table knows.
#[derive(Clone, Debug, Default)]
pub struct RestartTarget {
    pub ok: bool,
    /// "shell", "app", "service" or "process".
    pub kind: &'static str,
    pub name: String,
    pub icon: String,
    pub pid: Option<i64>,
    /// The process of an app that was selected ("'firefox' belongs to Firefox").
    pub via: Option<String>,
    pub reason: String,
}

/// actions.js restartTargetOf: an app row -> the app's main process; a process of an app (Details, an expanded
/// app) -> that process (the host decides whether the app restarts or only that process).
pub fn restart_target_of(row: Option<&ProcRow>) -> RestartTarget {
    let Some(row) = row else { return RestartTarget { reason: tr("restartOne").into(), ..Default::default() } };
    if is_shell_row(row) {
        return RestartTarget { ok: true, kind: "shell", name: "Win11 Shell".into(), ..Default::default() };
    }
    if row.multi {
        return RestartTarget { reason: tr("restartOne").into(), ..Default::default() };
    }
    let app: Option<&ProcRow> = if row.kind == "app" {
        Some(row)
    } else if let Some(p) = row.parent.as_deref().filter(|p| p.kind == "app") {
        Some(p)
    } else {
        row.app_row.as_deref()
    };
    if let Some(app) = app {
        let roots = roots_of(&app.procs);
        let root = roots.first();
        let own_pids: Vec<i64> = if row.kind == "app" { Vec::new() } else if row.pids.is_empty() { row.procs.iter().filter_map(pid_of).collect() } else { row.pids.clone() };
        let own: Option<Value> = if own_pids.len() == 1 {
            Some(row.procs.iter().find(|p| pid_of(p) == Some(own_pids[0])).cloned().unwrap_or_else(|| json!({ "pid": own_pids[0] })))
        } else {
            None
        };
        let pid = own.as_ref().and_then(pid_of).or_else(|| root.and_then(pid_of)).or_else(|| app.pids.first().copied());
        let ok = pid.is_some()
            && match (&own, root) {
                (Some(o), _) => o["canKill"] != false,
                (None, Some(r)) => r["canKill"] != false,
                (None, None) => app.can_kill,
            };
        let via = if row.kind == "app" || app.pids.len() <= 1 || root.is_some_and(|r| pid_of(r) == pid) {
            None
        } else if !row.name.is_empty() {
            Some(row.name.clone())
        } else if !row.display_name.is_empty() {
            Some(row.display_name.clone())
        } else {
            None
        };
        return RestartTarget {
            ok,
            kind: "app",
            name: app.name.clone(),
            icon: app.icon.clone(),
            pid,
            via,
            reason: if ok { String::new() } else { tr("restartSystem").into() },
        };
    }
    let tg = target_of(row);
    if !tg.can_kill {
        return RestartTarget { reason: tr("restartSystem").into(), ..Default::default() };
    }
    let gkey = tg.procs.first().map(|p| util::text(&p["groupKey"])).unwrap_or_default();
    let unit = gkey.starts_with("unit:") && gkey.ends_with(".service") && gkey.len() > "unit:.service".len() && tg.procs.iter().all(|p| p["groupKey"] == gkey.as_str());
    let roots = roots_of(&tg.procs);
    if roots.len() > 1 && !unit {
        return RestartTarget { reason: tr("restartPickOne").into(), ..Default::default() };
    }
    let pid = roots.first().and_then(pid_of).or_else(|| if tg.pids.len() == 1 { Some(tg.pids[0]) } else { None });
    let Some(pid) = pid else { return RestartTarget { reason: tr("restartOne").into(), ..Default::default() } };
    RestartTarget { ok: true, kind: if unit { "service" } else { "process" }, name: tg.name, icon: tg.icon, pid: Some(pid), via: None, reason: String::new() }
}

/// "Win11 Shell opnieuw starten" (the banner, the menu and the command bar).
pub fn restart_shell(app: &Rc<App>) {
    let h = hostcall::health();
    if h.state() == State::Restarting {
        return;
    }
    let pid = shell_pid(app.store.get("processes").as_deref());
    let app = app.clone();
    h.restart_shell(pid, move |r| match r {
        Ok(("dryRun", _)) => app.toast_info(tr("shellDryRun"), ""),
        Ok(_) => {}
        Err(e) => {
            let msg = if e.code == "unknown_method" { tr("restartUnsupported").to_owned() } else { message_of(&e) };
            app.alert_error(tr("shellFailed"), &msg, || {});
        }
    });
}

fn restart_failed(app: &Rc<App>, name: &str, e: &kit::CallError) {
    let title = fill(tr("restartFailed"), &[("name", name)]);
    let msg = if e.code == "unknown_method" { tr("restartUnsupported").to_owned() } else { message_of(e) };
    if hostcall::is_transport(e) {
        app.toast_error(&title, &msg);
    } else {
        app.alert_error(&title, &msg, || {});
    }
}

/// Restart the app or process of a row: the host says what that means, Task Manager asks the matching question.
pub fn restart_task(app: &Rc<App>, row: &ProcRow) {
    restart_task_with(app, row, false);
}

/// `restart_task` that always asks first (the `open=restart` test helper).
pub fn ask_restart_task(app: &Rc<App>, row: &ProcRow) {
    restart_task_with(app, row, true);
}

fn restart_task_with(app: &Rc<App>, row: &ProcRow, always_ask: bool) {
    let target = restart_target_of(Some(row));
    if target.kind == "shell" {
        restart_shell(app);
        return;
    }
    let Some(pid) = target.pid.filter(|_| target.ok) else { return };
    let app = app.clone();
    hostcall::call_with("sysmon.restartPlan", json!({ "pid": pid }), Opts { timeout: Some(6000), ..Default::default() }, move |r| {
        let plan = match r {
            Ok(plan) => Some(plan),
            Err(e) if e.code == "unknown_method" => None,
            Err(e) => {
                restart_failed(&app, &target.name, &e);
                return;
            }
        };
        if plan.as_ref().is_some_and(|p| p["how"] == "shell") {
            restart_shell(&app);
            return;
        }
        let how: String = match plan.as_ref().and_then(|p| p["how"].as_str()) {
            Some(h) => h.to_owned(),
            None => if target.kind == "process" { "exec".into() } else { target.kind.into() },
        };
        let name = plan.as_ref().and_then(|p| p["name"].as_str()).filter(|s| !s.is_empty()).map(str::to_owned).unwrap_or_else(|| target.name.clone());
        let via = match &plan {
            Some(p) => p["via"].as_str().filter(|s| !s.is_empty()).map(str::to_owned),
            None => target.via.clone(),
        };
        let rt = RestartTarget { kind: if how == "exec" { "process" } else if how == "app" { "app" } else if how == "service" { "service" } else { "process" }, name, via, ..target };
        let title = fill(tr("restartTitle"), &[("name", &rt.name)]);
        if how == "app" {
            // always asked: the app closes with all its windows (work that was not saved can be lost)
            let text = match &rt.via {
                Some(v) => fill(tr("restartBodyApp"), &[("name", &rt.name), ("proc", v)]),
                None => fill(tr("restartBodyAppRow"), &[("name", &rt.name)]),
            };
            let body = dlg_body(icon_for(&rt.icon, 32), &[&text], Some(tr("restartHint")), None);
            let a = app.clone();
            Dialog::new(&title)
                .body(&body)
                .primary(tr("restart"))
                .close_label(tr("cancel"))
                .default_button(DialogResult::Close)
                .width(460)
                .show(&app.parent(), move |r| {
                    if r == DialogResult::Primary {
                        run_restart(&a, rt, pid, false);
                    }
                });
        } else if always_ask || !util::load_bool("restartNoConfirm", false) {
            let check = kit::controls::checkbox(tr("dontAsk"));
            let text = fill(tr(if how == "service" { "restartBodyService" } else { "restartBody" }), &[("name", &rt.name)]);
            let body = dlg_body(icon_for(&rt.icon, 32), &[&text], Some(tr("restartHint")), Some(&check));
            let a = app.clone();
            Dialog::new(&title)
                .body(&body)
                .primary(tr("restart"))
                .close_label(tr("cancel"))
                .default_button(DialogResult::Close)
                .width(460)
                .show(&app.parent(), move |r| {
                    if r != DialogResult::Primary {
                        return;
                    }
                    if check.is_active() {
                        util::save("restartNoConfirm", json!(true));
                    }
                    run_restart(&a, rt, pid, false);
                });
        } else {
            run_restart(&app, rt, pid, false);
        }
    });
}

fn run_restart(app: &Rc<App>, rt: RestartTarget, pid: i64, confirm: bool) {
    let busy = app.toast(Toast::new(&fill(tr("restarting"), &[("name", &rt.name)])).glyph("restart").timeout(0));
    let mut args = json!({ "pid": pid });
    if confirm {
        args["confirm"] = json!(true);
    }
    let app = app.clone();
    hostcall::call("sysmon.restartProcess", args, move |r| {
        busy.close();
        let res = match r {
            Ok(res) => res,
            Err(e) if e.code == "confirm_required" && !confirm => {
                // the host asks first for some processes (e.g. one that runs in a terminal): its own question
                let body = dlg_body(icon_for(&rt.icon, 32), &[&e.message], None, None);
                let a = app.clone();
                Dialog::new(&fill(tr("restartTitle"), &[("name", &rt.name)]))
                    .body(&body)
                    .primary(tr("restartAnyway"))
                    .close_label(tr("cancel"))
                    .default_button(DialogResult::Close)
                    .width(460)
                    .show(&app.parent(), move |r| {
                        if r == DialogResult::Primary {
                            run_restart(&a, rt, pid, true);
                        }
                    });
                return;
            }
            Err(e) => {
                restart_failed(&app, &rt.name, &e);
                app.store.refresh("processes");
                return;
            }
        };
        if res["dryRun"] == true {
            // the host's --dry-run says what it would have done (sysmon.restartProcess -> {how, name})
            let how = res["how"].as_str().unwrap_or("");
            let name = res["name"].as_str().filter(|s| !s.is_empty()).unwrap_or(&rt.name).to_owned();
            let plan = if ["exec", "app", "service", "shell"].contains(&how) { fill(tr(&format!("dry_{how}")), &[("name", &name)]) } else { String::new() };
            app.toast_info(tr("restartDryRun"), &plan);
        } else if res["ok"] == false {
            let msg = res["error"].as_str().filter(|s| !s.is_empty()).unwrap_or(tr("failedGeneric")).to_owned();
            let a = app.clone();
            app.alert_error(&fill(tr("restartFailed"), &[("name", &rt.name)]), &msg, move || a.store.refresh("processes"));
            return;
        } else if res["how"] == "shell" {
            hostcall::health().expect_restart(true);
        } else {
            app.toast_success(&fill(tr("restarted"), &[("name", &rt.name)]), 3500);
        }
        after(300, move || app.store.refresh("processes"));
    });
}

// ------------------------------------------------------------------------------------------------ efficiency

/// The toast with the leaf (the kit's toasts take glyph names only).
fn leaf_toast(app: &Rc<App>, title: &str) {
    app.toast(Toast::new(title).glyph("process").timeout(3000));
    let mut c = app.window.layer().first_child();
    while let Some(w) = c {
        if w.has_css_class("w11-toast-stack")
            && let Some(last) = w.last_child()
        {
            util::swap_glyph(&last, TmGlyph::LeafOutline);
        }
        c = w.next_sibling();
    }
}

/// Turn efficiency mode on or off for a row's processes (on is explained first unless "Niet meer vragen").
pub fn set_efficiency(app: &Rc<App>, row: &ProcRow, enable: bool) {
    set_efficiency_with(app, row, enable, false);
}

/// Turning efficiency mode on, always explained first (the `open=efficiency` test helper).
pub fn ask_efficiency(app: &Rc<App>, row: &ProcRow) {
    set_efficiency_with(app, row, true, true);
}

fn set_efficiency_with(app: &Rc<App>, row: &ProcRow, enable: bool, always_ask: bool) {
    let tg = target_of(row);
    if tg.pids.is_empty() {
        return;
    }
    let run = {
        let app = app.clone();
        move |tg: Target| {
            let a = app.clone();
            hostcall::call("sysmon.setEfficiency", json!({ "pids": tg.pids, "enabled": enable }), move |r| {
                match r {
                    Ok(res) => {
                        let results: Vec<Value> = res["results"].as_array().cloned().unwrap_or_default();
                        let failed: Vec<&Value> = results.iter().filter(|r| r["ok"] != true).collect();
                        if !failed.is_empty() && failed.len() == results.len() {
                            a.toast_error(tr("effFailed"), failed[0]["error"].as_str().unwrap_or(""));
                        } else {
                            leaf_toast(&a, &fill(tr(if enable { "effEnabled" } else { "effDisabled" }), &[("name", &tg.name)]));
                        }
                    }
                    Err(e) => a.toast_error(tr("effFailed"), &e.message),
                }
                a.store.refresh("processes");
            });
        }
    };
    if enable && (always_ask || !util::load_bool("effNoConfirm", false)) {
        let check = kit::controls::checkbox(tr("dontAsk"));
        let leaf = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        leaf.add_css_class("taskmgr-dlg-leaf");
        leaf.set_size_request(32, 32);
        leaf.set_hexpand(false);
        let img = util::icon(TmGlyph::LeafOutline, 28);
        img.set_hexpand(true);
        leaf.append(&img);
        let body = dlg_body(leaf.upcast(), &[&fill(tr("effBody"), &[("name", &tg.name)])], None, Some(&check));
        Dialog::new(&fill(tr("effTitle"), &[("name", &tg.name)]))
            .body(&body)
            .primary(tr("effOn"))
            .close_label(tr("cancel"))
            .default_button(DialogResult::Close)
            .width(460)
            .show(&app.parent(), move |r| {
                if r != DialogResult::Primary {
                    return;
                }
                if check.is_active() {
                    util::save("effNoConfirm", json!(true));
                }
                run(tg);
            });
        return;
    }
    run(tg);
}

// ------------------------------------------------------------------------------------------------- the rest

fn exe_of(tg: &Target) -> String {
    if tg.exe.starts_with('/') { tg.exe.clone() } else { util::exe_path(&tg.cmd) }
}

pub fn open_file_location(app: &Rc<App>, row: &ProcRow) {
    let exe = exe_of(&target_of(row));
    if exe.is_empty() {
        return;
    }
    let app = app.clone();
    hostcall::call("files.open", json!({ "path": util::dir_name(&exe) }), move |r| {
        if let Err(e) = r {
            app.toast_error(tr("fileLocation"), &e.message);
        }
    });
}

pub fn search_online(app: &Rc<App>, row: &ProcRow) {
    let tg = target_of(row);
    let proc_name = tg.procs.first().map(|p| util::text(&p["name"])).unwrap_or_default();
    let term = if !proc_name.is_empty() && proc_name != tg.name { format!("{} {proc_name}", tg.name) } else { tg.name.clone() };
    let engine = app.setting("search.webEngine").as_str().filter(|s| !s.is_empty()).unwrap_or("https://duckduckgo.com/?q=").to_owned();
    let uri = format!("{engine}{}", kit::backend::encode(&format!("{term} linux")));
    let app = app.clone();
    hostcall::call("apps.openUri", json!({ "uri": uri }), move |r| {
        if let Err(e) = r {
            app.toast_error(tr("searchOnline"), &e.message);
        }
    });
}

/// The properties dialog of a row.
pub fn show_properties(app: &Rc<App>, row: &ProcRow) {
    let tg = target_of(row);
    let p: Value = tg.procs.first().cloned().unwrap_or_else(|| json!({ "name": row.name, "pid": row.pid, "user": row.user }));
    let many = tg.procs.len() > 1;
    let exe = exe_of(&tg);
    let count = plural(tg.procs.len() as i64, tr("pCountOne"), tr("pCountOther"));
    let pid_text = match pid_of(&p) {
        Some(pid) => pid.to_string(),
        None => "\u{2013}".into(),
    };
    let grid = gtk::Grid::new();
    grid.add_css_class("taskmgr-props-grid");
    grid.set_column_spacing(20);
    grid.set_row_spacing(8);
    let mut row_i = 0;
    let mut kv = |k: &str, v: gtk::Widget| {
        let kl = util::label(k, "taskmgr-props-key");
        kl.set_valign(gtk::Align::Start);
        kl.set_ellipsize(gtk::pango::EllipsizeMode::None);
        grid.attach(&kl, 0, row_i, 1, 1);
        v.set_hexpand(true);
        grid.attach(&v, 1, row_i, 1, 1);
        row_i += 1;
    };
    let val = |s: &str| -> gtk::Widget {
        let l = util::wrap_label(s, "taskmgr-props-value");
        l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        l.set_max_width_chars(52);
        l.upcast()
    };
    let name = util::text(&p["name"]);
    kv(tr("pName"), val(if name.is_empty() { tg.name.as_str() } else { name.as_str() }));
    let display = util::text(&p["displayName"]);
    if !display.is_empty() && display != name {
        kv(tr("pDesc"), val(&display));
    }
    if many {
        kv(tr("pPids"), val(&count));
    } else {
        kv(tr("pPid"), val(&pid_text));
    }
    let user = util::text(&p["user"]);
    kv(tr("pUser"), val(if user.is_empty() { "\u{2013}" } else { user.as_str() }));
    kv(tr("pStatus"), val(tr(&format!("st_{}", norm_status(row.status)))));
    let started = match p["started"].as_f64().filter(|s| *s > 0.0) {
        Some(s) => {
            let h24 = kit::ctx().h24();
            format!(
                "{} {} ({})",
                kit::format::date(s, kit::format::DateFormat::Medium),
                kit::format::time(s, false, h24),
                kit::format::rel_time(s, kit::format::now_ms())
            )
        }
        None => "\u{2013}".into(),
    };
    kv(tr("pStarted"), val(&started));
    let (cpu, mem, disk) = if tg.procs.is_empty() {
        (row.cpu, row.memory, row.disk)
    } else {
        (
            tg.procs.iter().map(|x| util::num(&x["cpu"])).sum(),
            tg.procs.iter().map(|x| util::num(&x["memory"])).sum(),
            tg.procs.iter().map(|x| util::num(&x["diskRead"]) + util::num(&x["diskWrite"])).sum(),
        )
    };
    kv(tr("pCpu"), val(&util::cpu_pct(cpu)));
    kv(tr("pMem"), val(&util::mem_mb(mem)));
    kv(tr("pDisk"), val(&util::disk_mbs(disk)));
    let threads: f64 = tg.procs.iter().map(|x| util::num(&x["threads"])).sum();
    if threads > 0.0 {
        kv(tr("pThreads"), val(&kit::format::number(threads, 0)));
    }
    if !row.windows.is_empty() {
        kv(tr("pWindows"), val(&row.windows.join("\n")));
    }
    let loc = val(&if exe.is_empty() { tr("noLocation").to_owned() } else { util::dir_name(&exe) });
    if let Some(l) = loc.downcast_ref::<gtk::Label>() {
        l.set_selectable(true);
    }
    kv(tr("pLoc"), loc);
    let cmdwrap = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let cmd = gtk::Label::new(Some(if tg.cmd.is_empty() { "\u{2013}" } else { &tg.cmd }));
    cmd.add_css_class("taskmgr-props-cmd");
    cmd.add_css_class("w11-mono");
    cmd.set_wrap(true);
    cmd.set_wrap_mode(gtk::pango::WrapMode::Char);
    cmd.set_selectable(true);
    cmd.set_xalign(0.0);
    cmd.set_max_width_chars(52);
    let cmd_scroll = gtk::ScrolledWindow::new();
    cmd_scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    cmd_scroll.set_propagate_natural_height(true);
    cmd_scroll.set_max_content_height(96);
    cmd_scroll.set_child(Some(&cmd));
    cmd_scroll.add_css_class("taskmgr-props-cmdbox");
    cmdwrap.append(&cmd_scroll);
    let copy = gtk::Button::new();
    copy.add_css_class("w11-link");
    copy.add_css_class("taskmgr-props-copy");
    copy.set_halign(gtk::Align::Start);
    let crow = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    crow.append(&kit::icons::glyph("copy", 14));
    let clabel = gtk::Label::new(Some(tr("copy")));
    crow.append(&clabel);
    copy.set_child(Some(&crow));
    let text = tg.cmd.clone();
    copy.connect_clicked(move |b| {
        b.clipboard().set_text(&text);
        clabel.set_label(tr("copied"));
    });
    cmdwrap.append(&copy);
    kv(tr("pCmd"), cmdwrap.upcast());

    let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
    content.add_css_class("taskmgr-props");
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    head.append(&icon_for(&tg.icon, 40));
    let names = gtk::Box::new(gtk::Orientation::Vertical, 0);
    names.set_valign(gtk::Align::Center);
    names.append(&util::label(&tg.name, "w11-subtitle"));
    names.append(&util::label(&if many { count.clone() } else { format!("PID {pid_text}") }, "w11-caption w11-text-2"));
    head.append(&names);
    content.append(&head);
    content.append(&grid);
    Dialog::new(&fill(tr("propsTitle"), &[("name", &tg.name)])).body(&content).close_label(tr("close")).width(560).show(&app.parent(), |_| {});
}

/// The context menu of selected rows (Processen passes its table for Expand / Collapse).
pub fn process_menu(app: &Rc<App>, row: &Rc<ProcRow>, table: Option<&TmTable>) -> (Vec<TopAction>, Vec<MenuItem>) {
    let tg = target_of(row);
    let user = app.user();
    let can_eff = if tg.procs.is_empty() { tg.can_kill } else { tg.procs.iter().all(|p| (!user.is_empty() && p["user"] == user.as_str()) || p["canKill"] == true) };
    let rt = restart_target_of(Some(row));
    let mut items: Vec<MenuItem> = Vec::new();
    if rt.kind == "shell" {
        // like "Windows Verkenner opnieuw starten": first in the menu of our own shell
        let a = app.clone();
        items.push(MenuItem::new(tr("restartShell"), move || restart_shell(&a)).glyph("restart").description(tr("shellTip")).name("taskmgr-menu-restart-shell"));
        items.push(MenuItem::separator());
    }
    if let Some(t) = table
        && t.has_kids(&row.key)
    {
        let open = t.is_expanded(&row.key);
        let t2 = t.clone();
        let key = row.key.clone();
        items.push(MenuItem::new(tr(if open { "collapse" } else { "expand" }), move || t2.expand(&key, !open)).glyph(if open { "chevron-up" } else { "chevron-down" }));
        items.push(MenuItem::separator());
    }
    let (a, r) = (app.clone(), row.clone());
    let mut end = MenuItem::new(tr("endTask"), move || end_task(&a, &r)).glyph("process").accel("Del").enabled(tg.can_kill).name("taskmgr-menu-end");
    if !tg.can_kill {
        end = end.description(tr("systemProc"));
    }
    items.push(end);
    if rt.kind != "shell" {
        let (a, r) = (app.clone(), row.clone());
        let mut it = MenuItem::new(tr("restart"), move || restart_task(&a, &r)).glyph("restart").enabled(rt.ok).name("taskmgr-menu-restart");
        if !rt.ok {
            it = it.description(&rt.reason);
        }
        items.push(it);
    }
    let (a, r, on) = (app.clone(), row.clone(), tg.efficiency);
    let mut eff = MenuItem::new(tr("efficiency"), move || set_efficiency(&a, &r, !on)).enabled(can_eff).name("taskmgr-menu-efficiency");
    eff = if on { eff.checked(true) } else { eff.glyph("process") };
    items.push(eff);
    items.push(MenuItem::separator());
    let (a, pid) = (app.clone(), tg.pids.first().copied());
    items.push(MenuItem::new(tr("details"), move || a.navigate("details", json!({ "pid": pid }))).glyph("list").name("taskmgr-menu-details"));
    let (a, r) = (app.clone(), row.clone());
    items.push(MenuItem::new(tr("fileLocation"), move || open_file_location(&a, &r)).glyph("folder-open").enabled(!exe_of(&tg).is_empty()).name("taskmgr-menu-location"));
    let (a, r) = (app.clone(), row.clone());
    items.push(MenuItem::new(tr("searchOnline"), move || search_online(&a, &r)).glyph("globe").name("taskmgr-menu-search"));
    items.push(MenuItem::separator());
    let (a, r) = (app.clone(), row.clone());
    items.push(MenuItem::new(tr("properties"), move || show_properties(&a, &r)).glyph("properties").accel("Alt+Enter").name("taskmgr-menu-properties"));
    (Vec::new(), items)
}

/// The Task Manager glyphs of `process_menu`'s items (util::fix_menu_glyphs).
pub fn menu_glyphs(row: &ProcRow) -> Vec<(&'static str, TmGlyph)> {
    let mut out = vec![("taskmgr-menu-end", TmGlyph::EndTask)];
    if !target_of(row).efficiency {
        out.push(("taskmgr-menu-efficiency", TmGlyph::LeafOutline));
    }
    out
}

/// The icon of the efficiency command / menu item.
pub const LEAF: Icon = Icon::Tm(TmGlyph::LeafOutline);
/// The icon of the End task command / menu item.
pub const END: Icon = Icon::Tm(TmGlyph::EndTask);
