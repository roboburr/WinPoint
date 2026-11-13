//! Reviewed native software operations. Browser input never supplies commands or package names.
#[path = "software/appimage.rs"]
mod appimage;
#[path = "software/common.rs"]
mod common;
#[path = "software/flatpak.rs"]
mod flatpak;
#[path = "software/inspect.rs"]
mod inspect;
#[path = "software/packagekit.rs"]
mod packagekit;
#[path = "software/resolve.rs"]
mod resolve;
#[path = "software/snapd.rs"]
mod snapd;
#[path = "software/worker.rs"]
mod worker;
use super::ServiceContext;
use crate::files::{ApiError, ApiResult};
use common::*;
use glib::prelude::*;
use serde_json::{Value, json};
use std::{
    ffi::CString,
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::ffi::OsStrExt,
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};
#[derive(Clone)]
pub(crate) struct Paths {
    state: PathBuf,
    cache: PathBuf,
    plans: PathBuf,
    jobs: PathBuf,
    staging: PathBuf,
    operations: PathBuf,
    managed: PathBuf,
    applications: PathBuf,
}
#[derive(Clone)]
struct Tools {
    worker: PathBuf,
    dpkg_query: PathBuf,
    dpkg_deb: PathBuf,
    flatpak: PathBuf,
    snap_socket: PathBuf,
    unsquashfs: PathBuf,
}
impl Tools {
    fn system() -> Self {
        let worker = std::env::current_exe()
            .unwrap_or_default()
            .with_file_name("win11-software-worker");
        Self {
            worker,
            dpkg_query: "/usr/bin/dpkg-query".into(),
            dpkg_deb: "/usr/bin/dpkg-deb".into(),
            flatpak: "/usr/bin/flatpak".into(),
            snap_socket: "/run/snapd.socket".into(),
            unsquashfs: "/usr/bin/unsquashfs".into(),
        }
    }
}
pub(crate) struct Software {
    ctx: Arc<ServiceContext>,
    paths: Paths,
    tools: Tools,
    weak: Weak<Self>,
    workers: Mutex<Vec<JoinHandle<()>>>,
    stopping: AtomicBool,
}
pub(crate) fn run_worker() -> i32 {
    worker::main()
}
fn ioerr(e: impl std::fmt::Display) -> ApiError {
    ApiError::new("software_io", &e.to_string())
}
fn public(mut v: Value) -> Value {
    if let Some(m) = v.as_object_mut() {
        m.retain(|k, _| !k.starts_with('_'));
    }
    v
}
fn current_owner() -> Value {
    let pid = std::process::id();
    json!({"pid":pid,"start":process_start(pid)})
}
fn process_start(pid: u32) -> Option<String> {
    let s = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    s.rsplit_once(") ")?
        .1
        .split_whitespace()
        .nth(19)
        .map(str::to_owned)
}
fn owner_alive(v: &Value) -> bool {
    v["pid"]
        .as_u64()
        .and_then(|p| u32::try_from(p).ok())
        .and_then(process_start)
        .is_some_and(|s| v["start"] == s)
}
impl Software {
    pub(crate) fn new(ctx: Arc<ServiceContext>) -> Result<Arc<Self>, String> {
        let cache = ctx.cache_dir.join("software");
        let paths = Paths {
            state: ctx.config_dir.join("software"),
            plans: cache.join("plans"),
            jobs: cache.join("jobs"),
            staging: cache.join("staging"),
            operations: cache.join("operations"),
            cache,
            managed: glib::home_dir().join(".local/opt/win11shell-appimages"),
            applications: glib::user_data_dir().join("applications"),
        };
        for p in [
            &paths.state,
            &paths.cache,
            &paths.plans,
            &paths.jobs,
            &paths.staging,
            &paths.operations,
        ] {
            private_dir(p).map_err(|e| e.to_string())?;
        }
        Ok(Arc::new_cyclic(|weak| Self {
            ctx,
            paths,
            tools: Tools::system(),
            weak: weak.clone(),
            workers: Mutex::new(Vec::new()),
            stopping: AtomicBool::new(false),
        }))
    }
    pub(crate) fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
        if !method.starts_with("software.") {
            return None;
        }
        let lang = self.ctx.language();
        let keys = match method {
            "software.inspect" => &["path"][..],
            "software.uninstallInfo" => &["id"][..],
            "software.prepare" => &["action", "path", "id"][..],
            "software.execute" => &["planId"][..],
            "software.status" | "software.cancel" => &["jobId"][..],
            "software.jobs" => &[][..],
            _ => return None,
        };
        if !args.is_object()
            || args
                .as_object()
                .is_some_and(|o| o.keys().any(|k| !keys.contains(&k.as_str())))
        {
            return Some(Err(err(
                &lang,
                "bad_args",
                "Ongeldige invoer voor softwarebeheer.",
                "Invalid software management input.",
            )));
        }
        if self.stopping.load(Ordering::Acquire) {
            return Some(Err(err(
                &lang,
                "offline",
                "Softwarebeheer wordt afgesloten.",
                "Software management is shutting down.",
            )));
        }
        Some(match method {
            "software.inspect" => inspect::inspect(&args["path"], &lang, &self.tools).map(public),
            "software.uninstallInfo" => self
                .uninstall_info(args["id"].as_str().unwrap_or(""), &lang)
                .map(public),
            "software.prepare" => self.prepare(args, &lang),
            "software.execute" => self.execute(&args["planId"], &lang),
            "software.status" => self.status(&args["jobId"], &lang),
            "software.cancel" => self.cancel(&args["jobId"], &lang),
            "software.jobs" => self.jobs(&lang),
            _ => unreachable!(),
        })
    }
    fn uninstall_info(&self, id: &str, lang: &str) -> ApiResult {
        let known = self.ctx.call("apps.list", &json!({}))?;
        resolve::resolve(id, &known, &self.paths, &self.tools, lang)
    }
    fn cleanup(&self) -> Result<(), ApiError> {
        let _guard = lock(&self.paths.plans).map_err(ioerr)?;
        let active_plans = fs::read_dir(&self.paths.jobs)
            .map_err(ioerr)?
            .flatten()
            .filter_map(|e| read_json(&e.path(), 1 << 20).ok())
            .filter(|v| active(v) && owner_alive(&v["_owner"]))
            .filter_map(|v| v["planId"].as_str().map(str::to_owned))
            .collect::<std::collections::HashSet<_>>();
        for entry in fs::read_dir(&self.paths.plans).map_err(ioerr)?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(ident) = name
                .strip_suffix(".json")
                .filter(|s| token(&json!(s)).is_some())
            else {
                continue;
            };
            if let Ok(v) = read_json(&entry.path(), 1 << 20) {
                if now().saturating_sub(v["_created"].as_u64().unwrap_or(0)) > 600
                    && !active_plans.contains(ident)
                {
                    if let Some(stage) = v["_stage"].as_str().filter(|s| safe_stage(s)) {
                        let _ = fs::remove_file(self.paths.staging.join(stage));
                    }
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        let _jobs = lock(&self.paths.jobs).map_err(ioerr)?;
        let mut finished = fs::read_dir(&self.paths.jobs)
            .map_err(ioerr)?
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let ident = name
                    .strip_suffix(".json")
                    .filter(|s| token(&json!(s)).is_some())?
                    .to_string();
                let v = read_json(&e.path(), 1 << 20).ok()?;
                (!active(&v) || !owner_alive(&v["_owner"])).then_some((
                    v["_updated"].as_u64().unwrap_or(0),
                    ident,
                    e.path(),
                ))
            })
            .collect::<Vec<_>>();
        finished.sort_by_key(|(updated, _, _)| *updated);
        let extra = finished.len().saturating_sub(200);
        for (index, (updated, ident, path)) in finished.into_iter().enumerate() {
            if index < extra || now().saturating_sub(updated) > 30 * 86400 {
                let _ = fs::remove_file(path);
                let _ = fs::remove_file(self.paths.jobs.join(format!("{ident}.cancel")));
            }
        }
        drop(_jobs);
        // A host crash between capture and plan persistence leaves no authoritative plan. Only our generated, expired stages are pruned.
        let _capture = lock(&self.paths.staging).map_err(ioerr)?;
        for e in fs::read_dir(&self.paths.staging).map_err(ioerr)?.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !safe_stage(&name) {
                continue;
            }
            let ident = name.split('.').next().unwrap_or("");
            let old = e
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|d| d.as_secs() > 600);
            if old
                && !active_plans.contains(ident)
                && !self.paths.plans.join(format!("{ident}.json")).exists()
            {
                let _ = fs::remove_file(e.path());
            }
        }
        Ok(())
    }
    fn capture(&self, raw: &Value, ident: &str, lang: &str) -> Result<(PathBuf, Value), ApiError> {
        let (path, mut input, before) = source(raw, lang)?;
        let _capture_lock = lock(&self.paths.staging).map_err(ioerr)?;
        let mut count = 0usize;
        let mut bytes = 0u64;
        for e in fs::read_dir(&self.paths.staging).map_err(ioerr)?.flatten() {
            if e.file_name().to_str().is_some_and(safe_stage) {
                count += 1;
                bytes = bytes.saturating_add(e.metadata().map_err(ioerr)?.len());
            }
        }
        let size = before["size"].as_u64().unwrap_or(0);
        if count >= 20 || bytes.saturating_add(size) > (8u64 << 30) {
            return Err(err(
                lang,
                "staging_full",
                "Er staan te veel installatievoorstellen klaar. Rond ze af of wacht tien minuten.",
                "Too many installation plans are pending. Complete them or wait ten minutes.",
            ));
        }
        let cpath = CString::new(self.paths.staging.as_os_str().as_bytes()).map_err(ioerr)?;
        let mut stat = unsafe { std::mem::zeroed::<libc::statvfs>() };
        if unsafe { libc::statvfs(cpath.as_ptr(), &mut stat) } == 0
            && (stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64)
                < size.saturating_add(64 << 20)
        {
            return Err(err(
                lang,
                "no_space",
                "Er is niet genoeg vrije ruimte om dit pakket veilig klaar te zetten.",
                "There is not enough free space to stage this package safely.",
            ));
        }

        let suffix = path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        let suffix =
            if ["appimage", "deb", "snap", "flatpak", "flatpakref"].contains(&suffix.as_str()) {
                suffix
            } else {
                "appimage".into()
            };
        let stage = self.paths.staging.join(format!("{ident}.{suffix}"));
        let result = (|| {
            let mut out = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
                .open(&stage)
                .map_err(ioerr)?;
            let mut sum = glib::Checksum::new(glib::ChecksumType::Sha256)
                .ok_or_else(|| ioerr("SHA256 unavailable"))?;
            let mut buffer = [0u8; 128 * 1024];
            let mut size = 0u64;
            loop {
                let n = input.read(&mut buffer).map_err(ioerr)?;
                if n == 0 {
                    break;
                }
                size += n as u64;
                if size > MAX_SOURCE {
                    return Err(ioerr("The source changed while reading"));
                }
                out.write_all(&buffer[..n]).map_err(ioerr)?;
                sum.update(&buffer[..n]);
            }
            if size != before["size"].as_u64().unwrap_or(0) {
                return Err(ioerr("The source changed while reading"));
            }
            out.sync_all().map_err(ioerr)?;
            drop(out);
            let mut info = inspect::inspect(&json!(stage), lang, &self.tools)?;
            if ["appimage", "flatpak"].contains(&info["format"].as_str().unwrap_or("")) {
                info["name"] = json!(clean(
                    &path.file_stem().unwrap_or_default().to_string_lossy(),
                    200
                ));
            }
            info["sourcePath"] = json!(path);
            info["_sha256"] = json!(sum.string().map(|s| s.to_string()).unwrap_or_default());
            Ok((stage.clone(), info))
        })();
        if result.is_err() {
            let _ = fs::remove_file(stage);
        }
        result
    }
    fn prepare(&self, args: &Value, lang: &str) -> ApiResult {
        self.cleanup()?;
        let ident = id().map_err(ioerr)?;
        let (action, info, stage) = match args["action"].as_str() {
            Some("install") => {
                if !args["id"].is_null() {
                    return Err(err(
                        lang,
                        "bad_args",
                        "Kies één installatiebestand.",
                        "Choose one installation file.",
                    ));
                }
                let (stage, info) = self.capture(&args["path"], &ident, lang)?;
                ("install".to_string(), info, Some(stage))
            }
            Some("uninstall") => {
                if !args["path"].is_null() {
                    return Err(err(
                        lang,
                        "bad_args",
                        "Kies één geïnstalleerde app.",
                        "Choose one installed application.",
                    ));
                }
                let info = self.uninstall_info(args["id"].as_str().unwrap_or(""), lang)?;
                (
                    info["action"].as_str().unwrap_or("uninstall").to_string(),
                    info,
                    None,
                )
            }
            _ => {
                return Err(err(
                    lang,
                    "bad_args",
                    "Ongeldige softwareactie.",
                    "Invalid software action.",
                ));
            }
        };
        let allowed = info["supported"] == true;
        let mut plan = json!({"planId":if allowed{json!(ident)}else{Value::Null},"action":action,"name":info["name"],"provider":info["provider"],"scope":info["scope"],"description":info["description"],"requiresAuth":info["scope"]=="system"&&info["provider"]!="shortcut","canExecute":allowed,"notes":info["notes"].as_array().cloned().unwrap_or_default(),"info":public(info.clone()),"_info":info,"_created":now(),"_used":false,"_stage":stage.as_ref().and_then(|p|p.file_name()).map(|s|s.to_string_lossy().into_owned())});
        for k in ["sourcePath", "reasonCode", "reason"] {
            if !plan["_info"][k].is_null() {
                plan[k] = plan["_info"][k].clone()
            }
        }
        if allowed {
            if let Err(e) = write_json(&self.paths.plans.join(format!("{ident}.json")), &plan) {
                if let Some(path) = &stage {
                    let _ = fs::remove_file(path);
                }
                return Err(ioerr(e));
            }
        } else if let Some(path) = stage {
            let _ = fs::remove_file(path);
        }
        Ok(public(plan))
    }
    fn execute(&self, raw: &Value, lang: &str) -> ApiResult {
        let ident = token(raw).ok_or_else(|| {
            err(
                lang,
                "bad_args",
                "Dit installatievoorstel is ongeldig.",
                "This installation plan is invalid.",
            )
        })?;
        let operation = try_lock(&self.paths.operations).map_err(|_| {
            err(
                lang,
                "busy",
                "Er wordt al software gewijzigd. Wacht tot die actie is voltooid.",
                "Another software operation is active. Wait for it to finish.",
            )
        })?;
        let _guard = lock(&self.paths.plans).map_err(ioerr)?;
        let path = self.paths.plans.join(format!("{ident}.json"));
        let mut plan = read_json(&path, 1 << 20).map_err(|_| {
            err(
                lang,
                "not_found",
                "Dit voorstel bestaat niet meer. Kies de app opnieuw.",
                "This plan is no longer available. Select the application again.",
            )
        })?;
        if plan["_used"] == true
            || now().saturating_sub(plan["_created"].as_u64().unwrap_or(0)) > 600
        {
            return Err(err(
                lang,
                "expired",
                "Dit voorstel is verlopen of al gebruikt. Kies de app opnieuw.",
                "This plan expired or was already used. Select the application again.",
            ));
        }
        if plan["action"] != "install" {
            let current = self.uninstall_info(plan["_info"]["id"].as_str().unwrap_or(""), lang)?;
            if current != plan["_info"] {
                return Err(err(
                    lang,
                    "changed",
                    "Deze app is gewijzigd. Controleer het voorstel opnieuw.",
                    "This application changed. Review a new plan.",
                ));
            }
        }
        let job_id = id().map_err(ioerr)?;
        let job = json!({"jobId":job_id,"planId":ident,"action":plan["action"],"name":plan["name"],"provider":plan["provider"],"state":"queued","createdAt":now(),"progress":Value::Null,"cancellable":true,"cancelRequested":false,"message":t(lang,"Wachten op starten…","Waiting to start…"),"dryRun":self.ctx.dry_run||crate::backend::dry_run(),"_owner":current_owner(),"_updated":now()});
        plan["_used"] = json!(true);
        write_json(&path, &plan).map_err(ioerr)?;
        write_json(&self.paths.jobs.join(format!("{job_id}.json")), &job).map_err(ioerr)?;
        let this = self
            .weak
            .upgrade()
            .ok_or_else(|| ioerr("Software service unavailable"))?;
        let lang = lang.to_string();
        let jid = job_id.clone();
        let mut workers = self.workers.lock().unwrap_or_else(|e| e.into_inner());
        workers.retain(|h| !h.is_finished());
        workers.push(std::thread::spawn(move || {
            let _operation = operation;
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(||this.run(&jid,plan,&lang))).is_err(){this.update(&jid,json!({"state":"interrupted","cancellable":false,"message":t(&lang,"De actie is onverwacht onderbroken. Controleer de geïnstalleerde app voordat je opnieuw probeert.","The operation was interrupted unexpectedly. Check the installed application before retrying."),"error":{"code":"outcome_unknown","message":"The software worker stopped unexpectedly"}}));}
        }));
        self.ctx.emit("software.changed", public(job.clone()));
        Ok(public(job))
    }
    fn job_path(&self, raw: &Value, lang: &str) -> Result<PathBuf, ApiError> {
        let id = token(raw).ok_or_else(|| {
            err(
                lang,
                "bad_args",
                "Ongeldige softwaretaak.",
                "Invalid software job.",
            )
        })?;
        Ok(self.paths.jobs.join(format!("{id}.json")))
    }
    fn status(&self, raw: &Value, lang: &str) -> ApiResult {
        let path = self.job_path(raw, lang)?;
        let _guard = lock(&self.paths.jobs).map_err(ioerr)?;
        let mut v = read_json(&path, 1 << 20).map_err(|_| {
            err(
                lang,
                "not_found",
                "Deze softwaretaak is niet gevonden.",
                "This software job was not found.",
            )
        })?;
        if active(&v) && !owner_alive(&v["_owner"]) {
            v["state"] = json!("interrupted");
            v["cancellable"] = json!(false);
            v["message"] = json!(t(
                lang,
                "De app werd afgesloten voordat het resultaat bekend was. Controleer de geïnstalleerde app voordat je opnieuw probeert.",
                "The application closed before the result was known. Check the installed application before retrying."
            ));
            write_json(&path, &v).map_err(ioerr)?;
        }
        Ok(public(v))
    }
    fn cancel(&self, raw: &Value, lang: &str) -> ApiResult {
        self.status(raw, lang)?;
        let path = self.job_path(raw, lang)?;
        let _guard = lock(&self.paths.jobs).map_err(ioerr)?;
        let mut v = read_json(&path, 1 << 20).map_err(ioerr)?;
        if !active(&v) || v["cancellable"] != true {
            return Err(err(
                lang,
                "cannot_cancel",
                "Deze actie kan nu niet worden geannuleerd.",
                "This operation cannot be cancelled now.",
            ));
        }
        let ident = token(raw).unwrap();
        atomic(
            &self.paths.jobs.join(format!("{ident}.cancel")),
            b"cancel",
            0o600,
        )
        .map_err(ioerr)?;
        v["cancelRequested"] = json!(true);
        v["message"] = json!(t(
            lang,
            "Annuleren aangevraagd. Wachten op de pakketbeheerder…",
            "Cancellation requested. Waiting for the package manager…"
        ));
        v["_updated"] = json!(now());
        write_json(&path, &v).map_err(ioerr)?;
        let v = public(v);
        self.ctx.emit("software.changed", v.clone());
        Ok(v)
    }
    fn jobs(&self, lang: &str) -> ApiResult {
        let mut entries = fs::read_dir(&self.paths.jobs)
            .map_err(ioerr)?
            .flatten()
            .filter_map(|e| {
                e.file_name()
                    .to_str()
                    .and_then(|s| s.strip_suffix(".json"))
                    .map(str::to_string)
            })
            .filter(|s| token(&json!(s)).is_some())
            .filter_map(|s| self.status(&json!(s), lang).ok())
            .collect::<Vec<_>>();
        entries.sort_by_key(|v| {
            std::cmp::Reverse((
                v["createdAt"].as_u64().unwrap_or(0),
                v["jobId"].as_str().unwrap_or("").to_string(),
            ))
        });
        entries.truncate(100);
        Ok(json!(entries))
    }
    fn update(&self, id: &str, patch: Value) {
        if let Ok(_g) = lock(&self.paths.jobs) {
            let path = self.paths.jobs.join(format!("{id}.json"));
            if let Ok(mut v) = read_json(&path, 1 << 20) {
                if patch
                    .as_object()
                    .is_some_and(|m| m.iter().all(|(k, value)| v.get(k) == Some(value)))
                {
                    return;
                }
                if let (Some(a), Some(b)) = (v.as_object_mut(), patch.as_object()) {
                    for (k, v) in b {
                        a.insert(k.clone(), v.clone());
                    }
                }
                v["_updated"] = json!(now());
                if write_json(&path, &v).is_ok() {
                    self.ctx.emit("software.changed", public(v));
                }
            }
        }
    }
    fn cancelled(&self, id: &str) -> bool {
        self.paths.jobs.join(format!("{id}.cancel")).exists()
    }
    fn run(&self, id: &str, plan: Value, lang: &str) {
        self.update(
            id,
            json!({"state":"running","message":t(lang,"Software wijzigen…","Changing software…")}),
        );
        let mut result = self.perform(id, &plan, lang);
        if result
            .as_ref()
            .is_err_and(|e| e == "cancelled_copy" || e == "cancelled_upload")
        {
            result = Ok(None);
        }
        match result{Ok(Some(result))=>self.update(id,json!({"state":"completed","progress":100,"cancellable":false,"result":result,"message":if self.ctx.dry_run||crate::backend::dry_run(){t(lang,"Voorbeeld voltooid. Er is niets geïnstalleerd of verwijderd.","Preview completed. Nothing was installed or removed.")}else{t(lang,"Voltooid.","Completed.")}})),Ok(None)=>self.update(id,json!({"state":"cancelled","cancellable":false,"message":t(lang,"Geannuleerd.","Cancelled.")})),Err(detail)=>{let unknown=detail.contains("outcome_unknown")||detail.contains("result is unknown");let message=t(lang,"De actie is niet voltooid. De pakketbeheerder geeft meer informatie hieronder.","The operation did not complete. The package manager provides more information below.");self.update(id,json!({"state":if unknown{"interrupted"}else{"failed"},"cancellable":false,"message":message,"error":{"code":if unknown{"outcome_unknown"}else{"provider_error"},"message":clean(&detail,4000)}}));}}
        if let Some(stage) = plan["_stage"].as_str().filter(|s| safe_stage(s)) {
            let _ = fs::remove_file(self.paths.staging.join(stage));
        }
        let _ = fs::remove_file(self.paths.jobs.join(format!("{id}.cancel")));
        self.ctx.emit("apps.changed", json!({}));
    }
    fn perform(&self, id: &str, plan: &Value, lang: &str) -> Result<Option<Value>, String> {
        if self.cancelled(id) {
            return Ok(None);
        }
        if self.ctx.dry_run || crate::backend::dry_run() {
            return Ok(Some(json!({"dryRun":true})));
        }
        let info = &plan["_info"];
        let action = plan["action"].as_str().ok_or("Invalid action")?;
        let stage = plan["_stage"]
            .as_str()
            .filter(|s| safe_stage(s))
            .map(|s| self.paths.staging.join(s));
        if action == "install" {
            let (_, f, _) = source(
                &json!(stage.as_ref().ok_or("Missing staged package")?),
                lang,
            )
            .map_err(|e| e.message)?;
            if hash(f, None).map_err(|e| e.to_string())?.as_str()
                != info["_sha256"].as_str().unwrap_or("")
            {
                return Err("The reviewed installation file changed".into());
            }
        }
        let cancel = || self.cancelled(id);
        let progress =
            |n: Option<u32>, can: bool| self.update(id, json!({"progress":n,"cancellable":can}));
        match info["provider"].as_str() {
            Some("appimage") => {
                if action == "install" {
                    let flag = AtomicBool::new(cancel());
                    let record = appimage::install(
                        &self.paths,
                        stage.as_ref().ok_or("Missing staged package")?,
                        info,
                        &flag,
                        &|p| {
                            flag.store(cancel(), Ordering::Release);
                            progress(Some(p), true)
                        },
                    )?;
                    Ok(Some(json!({"desktopId":record["desktopId"]})))
                } else {
                    self.update(id, json!({"cancellable":false}));
                    appimage::remove(&self.paths, &info["_record"])?;
                    Ok(Some(json!({})))
                }
            }
            Some("shortcut") => {
                self.update(id, json!({"cancellable":false}));
                resolve::remove_shortcut(info)?;
                Ok(Some(json!({})))
            }
            Some("flatpak") => {
                match flatpak::run(&self.tools, action, info, stage.as_deref(), &cancel, &|| {})? {
                    flatpak::Outcome::Done => Ok(Some(json!({}))),
                    flatpak::Outcome::Cancelled => Ok(None),
                }
            }
            Some("snapd") => {
                self.update(id,json!({"state":"authenticating","message":t(lang,"Bevestig de actie in het systeemvenster indien gevraagd.","Confirm the operation in the system dialog if requested.")}));
                let change = if action == "install" {
                    snapd::install(
                        &self.tools.snap_socket,
                        stage.as_ref().ok_or("Missing staged package")?,
                        info["confinement"].as_str().unwrap_or("strict"),
                        &cancel,
                        &|n| progress(Some(n), true),
                    )?
                } else {
                    snapd::remove(
                        &self.tools.snap_socket,
                        info["_package"].as_str().ok_or("Missing snap name")?,
                    )?
                };
                self.update(id, json!({"state":"running"}));
                match snapd::wait(&self.tools.snap_socket, &change, &cancel, &progress)? {
                    snapd::Outcome::Done => Ok(Some(json!({}))),
                    snapd::Outcome::Cancelled => Ok(None),
                }
            }
            Some("packagekit") => {
                let update = |state: &str, p: Option<u32>, can: bool| {
                    self.update(id, json!({"state":state,"progress":p,"cancellable":can}))
                };
                let (method, args) = if action == "install" {
                    (
                        "InstallFiles",
                        (
                            4u64,
                            vec![
                                stage
                                    .as_ref()
                                    .ok_or("Missing staged package")?
                                    .to_string_lossy()
                                    .into_owned(),
                            ],
                        )
                            .to_variant(),
                    )
                } else {
                    let pkg = info["_package"]
                        .as_str()
                        .filter(|s| package(s) && !protected(s))
                        .ok_or("Invalid package")?;
                    let resolved = packagekit::resolve(
                        pkg.split(':').next().unwrap_or(pkg),
                        info["_architecture"].as_str().unwrap_or(""),
                    )?;
                    (
                        "RemovePackages",
                        (4u64, vec![resolved], false, false).to_variant(),
                    )
                };
                let planned = match packagekit::run(method, &args, &cancel, &update)? {
                    packagekit::Outcome::Cancelled => return Ok(None),
                    packagekit::Outcome::Done(p) => p,
                };
                validate_removals(
                    &planned,
                    action,
                    info["_package"].as_str().unwrap_or(""),
                    lang,
                )?;
                if cancel() {
                    return Ok(None);
                }
                let actual = if method == "InstallFiles" {
                    (
                        0u64,
                        args.child_value(1)
                            .get::<Vec<String>>()
                            .ok_or("Invalid package paths")?,
                    )
                        .to_variant()
                } else {
                    (
                        0u64,
                        args.child_value(1)
                            .get::<Vec<String>>()
                            .ok_or("Invalid package ids")?,
                        false,
                        false,
                    )
                        .to_variant()
                };
                match packagekit::run(method, &actual, &cancel, &update)? {
                    packagekit::Outcome::Done(_) => Ok(Some(json!({}))),
                    packagekit::Outcome::Cancelled => Ok(None),
                }
            }
            _ => Err("Unsupported package manager".into()),
        }
    }
    pub(crate) fn shutdown(&self) {
        self.stopping.store(true, Ordering::Release);
        for handle in self
            .workers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
        {
            let _ = handle.join();
        }
    }
}
fn active(v: &Value) -> bool {
    ["queued", "running", "authenticating"].contains(&v["state"].as_str().unwrap_or(""))
}
fn safe_stage(s: &str) -> bool {
    s.split_once('.').is_some_and(|(id, ext)| {
        token(&json!(id)).is_some()
            && ["appimage", "deb", "snap", "flatpak", "flatpakref"].contains(&ext)
    })
}

fn validate_removals(
    planned: &[(u32, String, String)],
    action: &str,
    selected: &str,
    lang: &str,
) -> Result<(), String> {
    for (info_kind, pkg, _) in planned {
        let name = pkg.split(';').next().unwrap_or("");
        if [13, 28, 29].contains(info_kind)
            && (protected(name)
                || action == "install"
                || name != selected.split(':').next().unwrap_or(""))
        {
            return Err(t(lang,"De pakketbeheerder wil andere of beschermde pakketten verwijderen. Deze actie is gestopt.","The package manager wants to remove other or protected packages. This operation was stopped.").into());
        }
    }
    Ok(())
}
