//! Native Task Manager service, retaining the original sysmon wire contract.
//! Process sampling uses private resident memory and machine-share CPU; no Python is invoked.
use super::ServiceContext;
use crate::files::{ApiError, ApiResult};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
#[path = "sysmon/actions.rs"]
mod actions;
#[path = "sysmon/common.rs"]
mod common;
#[path = "sysmon/health.rs"]
mod health;
#[path = "sysmon/performance.rs"]
mod performance;
#[path = "sysmon/process.rs"]
mod process;
#[path = "sysmon/startup.rs"]
mod startup;

pub(crate) struct Sysmon {
    ctx: Arc<ServiceContext>,
    process: Mutex<process::ProcessState>,
    performance: Mutex<performance::Performance>,
    gpu: Arc<performance::Gpu>,
    actions: Mutex<actions::ActionState>,
    stopped: AtomicBool,
}
fn shared_gpu() -> Arc<performance::Gpu> {
    static GPU: OnceLock<Arc<performance::Gpu>> = OnceLock::new();
    GPU.get_or_init(performance::Gpu::new).clone()
}
pub(super) fn gpu_names() -> Vec<String> {
    performance::gpu_names(&shared_gpu())
}
impl Sysmon {
    pub(crate) fn new(ctx: Arc<ServiceContext>) -> Result<Arc<Self>, String> {
        let service = Arc::new(Self {
            ctx,
            process: Mutex::new(process::ProcessState::default()),
            performance: Mutex::new(performance::Performance::new()),
            gpu: shared_gpu(),
            actions: Mutex::new(actions::ActionState::default()),
            stopped: AtomicBool::new(false),
        });
        let weak = Arc::downgrade(&service);
        std::thread::spawn(move || {
            if let Some(s) = weak.upgrade() {
                s.process.lock().unwrap_or_else(|e| e.into_inner()).sample();
            }
            loop {
                std::thread::sleep(Duration::from_secs(5));
                let Some(service) = weak.upgrade() else { break };
                if service.stopped.load(Ordering::Relaxed) {
                    break;
                }
                if service.ctx.subscriber_count() == 0 {
                    continue;
                }
                service
                    .process
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .sample();
            }
        });
        Ok(service)
    }
    pub(crate) fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
        if !method.starts_with("sysmon.") {
            return None;
        }
        let result = match method {
            "sysmon.processes" => Ok(self
                .process
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .processes(&self.ctx, args["processes"] != false)),
            "sysmon.performance" => Ok(self
                .performance
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&self.ctx, &self.gpu)),
            "sysmon.startup" => Ok(startup::list(
                &self.ctx.language(),
                common::truth(&args["all"]),
            )),
            "sysmon.setStartup" if self.ctx.dry_run => Ok(json!({"ok":true,"dryRun":true})),
            "sysmon.setStartup" => startup::set(
                &args["id"],
                common::truth(&args["enabled"]),
                self.ctx.lang(),
            ),
            "sysmon.services" => Ok(actions::services(
                &mut self.actions.lock().unwrap_or_else(|e| e.into_inner()),
                &self.process.lock().unwrap_or_else(|e| e.into_inner()),
            )),
            "sysmon.serviceAction" if self.ctx.dry_run => Ok(json!({"ok":true,"dryRun":true})),
            "sysmon.serviceAction" => actions::service_action(
                &args["name"],
                &args["action"],
                &mut self.actions.lock().unwrap_or_else(|e| e.into_inner()),
                self.ctx.lang(),
            ),
            "sysmon.endTask" | "sysmon.setEfficiency" if self.ctx.dry_run => {
                let rows = args["pids"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|p| p.is_string() || p.is_i64() || p.is_u64() || p.is_boolean())
                    .take(2000)
                    .map(|p| json!({"pid":p,"ok":true,"error":null}))
                    .collect::<Vec<_>>();
                Ok(json!({"ok":true,"dryRun":true,"results":rows}))
            }
            "sysmon.endTask" => actions::pid_list(&args["pids"], self.ctx.lang()).map(|pids| {
                actions::end_task(
                    &pids,
                    common::truth(&args["force"]),
                    &self.process.lock().unwrap_or_else(|e| e.into_inner()),
                    self.ctx.lang(),
                )
            }),
            "sysmon.setEfficiency" => {
                actions::pid_list(&args["pids"], self.ctx.lang()).map(|pids| {
                    actions::efficiency(
                        &pids,
                        common::truth(&args["enabled"]),
                        &mut self.actions.lock().unwrap_or_else(|e| e.into_inner()),
                        &mut self.process.lock().unwrap_or_else(|e| e.into_inner()),
                        self.ctx.lang(),
                    )
                })
            }
            "sysmon.restartPlan" | "sysmon.restartProcess" => {
                let pids = actions::pid_list(&args["pid"], self.ctx.lang());
                pids.and_then(|pids| {
                    if pids.len() != 1 {
                        return Err(ApiError::tr(
                            "bad_args",
                            self.ctx.lang(),
                            "sysmon.process.single",
                            crate::i18n::NO_PARAMS,
                        ));
                    }
                    let mut pm = self.process.lock().unwrap_or_else(|e| e.into_inner());
                    let plan = actions::restart_plan(pids[0], &mut pm, &self.ctx)?;
                    drop(pm);
                    if method == "sysmon.restartPlan" {
                        Ok(plan.summary(pids[0]))
                    } else if self.ctx.dry_run {
                        actions::confirm(&plan, args["confirm"] == true, pids[0], self.ctx.lang())?;
                        let mut out = plan.summary(pids[0]);
                        out["ok"] = json!(true);
                        out["dryRun"] = json!(true);
                        Ok(out)
                    } else {
                        actions::restart(
                            pids[0],
                            plan,
                            args["confirm"] == true,
                            &self.ctx,
                            &mut self.actions.lock().unwrap_or_else(|e| e.into_inner()),
                        )
                    }
                })
            }
            "sysmon.health" => Ok(health::health(
                &self.ctx,
                &mut self.process.lock().unwrap_or_else(|e| e.into_inner()),
                &self.gpu,
            )),
            _ => return None,
        };
        Some(result)
    }
    pub(crate) fn shutdown(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }
}
// R2-B: the sysmon stem in English through its call sites (round2-gate §6), Dutch byte-identical, and the D-G9
// unit friendly names.
#[cfg(test)]
mod i18n_tests {
    use super::*;
    use crate::i18n::{self, Lang};
    use std::fs;

    fn context(root: &std::path::Path, lang: &str) -> Arc<ServiceContext> {
        let config = root.join(format!("config-{lang}"));
        fs::create_dir_all(&config).unwrap();
        fs::write(
            config.join("settings.json"),
            json!({"language":lang}).to_string(),
        )
        .unwrap();
        ServiceContext::new(
            crate::events::Events::new(&root.join(format!("events-{lang}"))).unwrap(),
            config,
            root.join("cache"),
            root.join("ui"),
            "taskmgr".into(),
            false,
        )
        .unwrap()
    }
    fn error(result: Option<ApiResult>) -> ApiError {
        result.expect("a sysmon method").expect_err("an error")
    }
    /// A pid that does not exist now (no /proc entry): check_own calls it gone.
    fn missing_pid() -> u32 {
        (4_000_000..4_000_100)
            .find(|p| !std::path::Path::new(&format!("/proc/{p}")).exists())
            .unwrap()
    }

    #[test]
    fn sysmon_errors_render_in_english_and_dutch_through_the_service() {
        let root =
            std::env::temp_dir().join(format!("win11-sysmon-i18n-{}", crate::events::nonce()));
        for (lang, single, unknown, none, startup, critical, gone) in [
            (
                "en",
                "Specify one process.",
                "Unknown service.",
                "No processes were specified.",
                "Unknown startup app.",
                "Your desktop needs this process, so it can’t be ended.",
                "This process no longer exists.",
            ),
            (
                "nl",
                "Geef één proces op.",
                "Onbekende service.",
                "Geen processen opgegeven.",
                "Onbekend opstartitem.",
                "Dit proces is nodig voor je bureaublad en kan niet worden beëindigd.",
                "Dit proces bestaat niet meer.",
            ),
        ] {
            let ctx = context(&root, lang);
            let svc = Sysmon::new(ctx.clone()).unwrap();
            let e = error(svc.call("sysmon.restartPlan", &json!({"pid":[1, 2]})));
            assert_eq!((e.code.as_str(), e.message.as_str()), ("bad_args", single));
            assert_eq!(e.key.as_deref(), Some("sysmon.process.single"));
            // an invalid unit name: refused before any systemctl call
            let e = error(svc.call(
                "sysmon.serviceAction",
                &json!({"name":"-x","action":"start"}),
            ));
            assert_eq!(e.message, unknown);
            assert_eq!(
                error(svc.call("sysmon.endTask", &json!({"pids":[]}))).message,
                none
            );
            let e = error(svc.call(
                "sysmon.setStartup",
                &json!({"id":"../x.desktop","enabled":true}),
            ));
            assert_eq!(e.message, startup);
            // pid 1: check_own refuses before any signal; the row is results[].error
            let rows = svc
                .call("sysmon.endTask", &json!({"pids":[1]}))
                .unwrap()
                .unwrap();
            assert_eq!(rows["results"][0]["error"], critical);
            // restartPlan of a process that is gone: the code follows the reason in both languages. 3b898e6
            // matched "bestaat niet" in check_own's text, which was Dutch in both languages; a plain conversion
            // would have given English "protected", so the reason (actions::Own) picks the code now (review m4)
            let e = error(svc.call("sysmon.restartPlan", &json!({"pid":missing_pid()})));
            assert_eq!((e.code.as_str(), e.message.as_str()), ("not_found", gone));
            assert_eq!(e.key.as_deref(), Some("sysmon.process.gone"));
            let e = error(svc.call("sysmon.restartPlan", &json!({"pid":1})));
            assert_eq!(
                (e.code.as_str(), e.message.as_str()),
                ("protected", critical)
            );
            svc.shutdown();
            ctx.events.shutdown();
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unit_friendly_names_have_english_rows_and_dutch_is_unchanged() {
        for (unit, nl, en) in [
            ("dconf.service", "Instellingendatabase", "Settings database"),
            ("ssh-agent.service", "SSH-sleutels", "SSH keys"),
            (
                "mpris-proxy.service",
                "Bluetooth-mediabediening",
                "Bluetooth media control",
            ),
            (
                "gnome-session-manager@ubuntu.service",
                "GNOME-sessiebeheer",
                "GNOME session manager",
            ),
            (
                "org.gnome.SettingsDaemon.Power.service",
                "GNOME-instellingen: energiebeheer",
                "GNOME settings: power management",
            ),
            (
                "org.gnome.SettingsDaemon.Unknown.service",
                "GNOME-instellingendienst",
                "GNOME settings daemon",
            ),
            (
                "gvfs-udisks2-volume-monitor.service",
                "Stationsbewaking (schijven)",
                "Drive monitor (disks)",
            ),
            (
                "gvfs-other-volume-monitor.service",
                "Stationsbewaking (other)",
                "Drive monitor (other)",
            ),
            (
                "snap.firefox.firefox.service",
                "Snap-dienst (firefox)",
                "Snap service (firefox)",
            ),
            (
                "org.gnome.Shell@wayland.service",
                "GNOME Shell",
                "GNOME Shell",
            ),
        ] {
            assert_eq!(
                common::unit_friendly("nl", unit).as_deref(),
                Some(nl),
                "{unit}"
            );
            assert_eq!(
                common::unit_friendly("en", unit).as_deref(),
                Some(en),
                "{unit}"
            );
        }
        assert_eq!(common::unit_friendly("en", "something.service"), None);
        assert_eq!(
            common::friendly("en", &["gsd-power"]).as_deref(),
            Some("GNOME settings daemon")
        );
        assert_eq!(
            common::friendly("nl", &["gsd-power"]).as_deref(),
            Some("GNOME-instellingendienst")
        );
        for key in actions::Own::ALL.iter().map(|o| o.key()) {
            assert!(i18n::has(key), "{key}");
        }
        assert_eq!(
            actions::Own::Gone.message(Lang::En),
            "This process no longer exists."
        );
    }

    #[test]
    fn durations_keep_their_dutch_and_english_forms() {
        for (sec, nl, en) in [
            (5., "5 seconden", "5 s"),
            (61., "1 minuut", "1 minute"),
            (125., "2 minuten", "2 min"),
            (3725., "1 u 2 min", "1 h 2 min"),
        ] {
            assert_eq!(common::fmt_duration("nl", sec), nl);
            assert_eq!(common::fmt_duration("en", sec), en);
        }
    }
}
