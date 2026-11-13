use super::{common::*, hardware::session_id, number_arg};
use crate::{
    files::{ApiError, ApiResult},
    services::ServiceContext,
};
use serde_json::{Value, json};
use std::{
    fs,
    process::{Child, Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};
fn compositor() -> &'static str {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        "hyprland"
    } else {
        "gnome"
    }
}
fn running(name: &str) -> bool {
    let uid = unsafe { libc::geteuid() };
    use std::os::unix::fs::MetadataExt;
    fs::read_dir("/proc").is_ok_and(|ds| {
        ds.flatten().any(|d| {
            d.file_name()
                .to_string_lossy()
                .chars()
                .all(|c| c.is_ascii_digit())
                && fs::metadata(d.path()).is_ok_and(|m| m.uid() == uid)
                && fs::read_to_string(d.path().join("comm")).is_ok_and(|s| s.trim() == name)
        })
    })
}
pub(super) fn action(action: &str, ctx: &ServiceContext) -> ApiResult {
    // design §4.9: refused while our update job commits or Ubuntu's release upgrader runs (never `lock`)
    if let Some(error) = crate::services::update::guard::power_blocked(ctx, action) {
        return Err(error);
    }
    let comp = compositor();
    let r = match action {
        "shutdown" => run(&["systemctl", "poweroff"], 15, false),
        "reboot" => run(&["systemctl", "reboot"], 15, false),
        "suspend" => run(&["systemctl", "suspend"], 15, false),
        "logout" => {
            if comp == "hyprland" && run(&["hyprctl", "dispatch", "exit"], 5, false).ok {
                return Ok(json!({"ok":true}));
            }
            if comp == "gnome" && available("gnome-session-quit") {
                run(
                    &["gnome-session-quit", "--logout", "--no-prompt"],
                    15,
                    false,
                )
            } else {
                let sid = session_id().ok_or_else(|| {
                    ApiError::tr(
                        "failed",
                        ctx.lang(),
                        "system.power.noSession",
                        crate::i18n::NO_PARAMS,
                    )
                })?;
                run(&["loginctl", "terminate-session", &sid], 15, false)
            }
        }
        "lock" => {
            if comp == "hyprland" && available("hyprlock") && !running("hypridle") {
                let mut cmd = Command::new("hyprlock");
                let conf = std::env::var_os("WIN11SHELL_HYPRLOCK_CONFIG")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| ctx.config_dir.join("hypr/hyprlock.conf"));
                if conf.is_file() {
                    cmd.arg("-c").arg(conf);
                }
                if crate::launch::spawn(cmd).is_ok() {
                    return Ok(json!({"ok":true}));
                }
            }
            if let Some(id) = session_id() {
                run(&["loginctl", "lock-session", &id], 10, false)
            } else {
                run(&["loginctl", "lock-session"], 10, false)
            }
        }
        _ => {
            return Err(ApiError::tr(
                "bad_args",
                ctx.lang(),
                "system.power.unknownAction",
                crate::i18n::NO_PARAMS,
            ));
        }
    };
    if !r.ok {
        let err = r.err.to_lowercase();
        if err.contains("inhibit") {
            return Err(ApiError::tr(
                "inhibited",
                ctx.lang(),
                "system.power.inhibited",
                crate::i18n::NO_PARAMS,
            ));
        }
        if err.contains("interactive authentication required") || err.contains("access denied") {
            return Err(ApiError::tr(
                "denied",
                ctx.lang(),
                "system.power.denied",
                crate::i18n::NO_PARAMS,
            ));
        }
        return Err(ApiError::tr(
            "failed",
            ctx.lang(),
            "system.power.failed",
            crate::i18n::NO_PARAMS,
        ));
    }
    Ok(json!({"ok":true}))
}
pub(super) fn open_updater(lang: crate::i18n::Lang) -> ApiResult {
    for args in [
        &["update-manager"][..],
        &["gnome-software", "--mode=updates"][..],
        &["snap-store", "--mode=updates"][..],
    ] {
        if available(args[0]) {
            let mut cmd = Command::new(args[0]);
            cmd.args(&args[1..]);
            if crate::launch::spawn(cmd).is_ok() {
                return Ok(json!({"ok":true,"app":args[0]}));
            }
        }
    }
    Err(ApiError::tr(
        "unsupported",
        lang,
        "system.updates.noApp",
        crate::i18n::NO_PARAMS,
    ))
}
pub(super) struct NightLight {
    state: Mutex<(bool, i64, Option<Child>)>,
}
impl NightLight {
    pub fn new() -> Self {
        Self {
            state: Mutex::new((false, 4500, None)),
        }
    }
    pub fn supported(&self) -> bool {
        available("hyprsunset") && compositor() == "hyprland"
    }
    pub fn get(&self) -> Value {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        json!({"supported":self.supported(),"enabled":state.0,"temperature":state.1,"tool":available("hyprsunset").then_some("hyprsunset")})
    }
    fn kill(state: &mut (bool, i64, Option<Child>)) {
        if let Some(mut p) = state.2.take() {
            if p.try_wait().ok().flatten().is_none() {
                unsafe { libc::kill(p.id() as i32, libc::SIGTERM) };
                let end = Instant::now() + Duration::from_secs(2);
                while p.try_wait().ok().flatten().is_none() && Instant::now() < end {
                    thread::sleep(Duration::from_millis(20));
                }
                if p.try_wait().ok().flatten().is_none() {
                    let _ = p.kill();
                }
                let _ = p.wait();
            }
        }
    }
    /// D-G2: the GNOME version has no hyprsunset, so night light is set in Ubuntu's Settings app. The text is the
    /// UI's own (display.js nightNA) in both languages; i18n_tests compares the two.
    pub(super) fn unsupported(lang: crate::i18n::Lang) -> ApiError {
        ApiError::tr(
            "unsupported",
            lang,
            "system.nightLight.unsupported",
            crate::i18n::NO_PARAMS,
        )
    }
    pub fn set(&self, enabled: bool, temp: &Value, lang: crate::i18n::Lang) -> ApiResult {
        if !self.supported() {
            return Err(Self::unsupported(lang));
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let temp = if temp.is_null() {
            state.1
        } else {
            number_arg(temp, lang)?.trunc().clamp(1000., 10000.) as i64
        };
        if state.2.is_none() && running("hyprsunset") {
            let args = if enabled {
                vec![
                    "hyprctl".to_owned(),
                    "hyprsunset".into(),
                    "temperature".into(),
                    temp.to_string(),
                ]
            } else {
                vec!["hyprctl".into(), "hyprsunset".into(), "identity".into()]
            };
            let r = run(
                &args.iter().map(String::as_str).collect::<Vec<_>>(),
                5,
                false,
            );
            if r.ok && r.out.to_lowercase().contains("ok") {
                state.0 = enabled;
                state.1 = temp;
                drop(state);
                return Ok(self.get());
            }
        }
        Self::kill(&mut state);
        if enabled {
            let child = Command::new("hyprsunset")
                .arg("-t")
                .arg(temp.to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|_| {
                    ApiError::tr(
                        "failed",
                        lang,
                        "system.nightLight.failed",
                        crate::i18n::NO_PARAMS,
                    )
                })?;
            state.2 = Some(child)
        }
        state.0 = enabled;
        state.1 = temp;
        drop(state);
        Ok(self.get())
    }
    pub fn stop(&self) {
        Self::kill(&mut self.state.lock().unwrap_or_else(|e| e.into_inner()));
    }
}

/// GW2c: the update service's power guard in `action` (design §4.9). The guard reads files and `/proc` only (no
/// seams here: the system paths, this boot's real boot id, this test process as the job's live owner).
#[cfg(test)]
mod update_guard_tests {
    use super::*;
    use crate::services::update::{guard, paths, store};

    /// A committing job of this boot whose owner (this process) is alive.
    fn committing_job(ctx: &ServiceContext) -> store::Store {
        let setup = paths::process_setup().expect("no WIN11_UPDATE_* seam in the unit sandbox");
        let boot = store::boot_id(&setup.paths).expect("a boot id");
        let s = store::Store::open(ctx.cache_dir.join("update")).unwrap();
        s.write(
            store::JOB,
            &json!({"version":1,"jobId":"0123456789abcdef0123456789abcdef","bootId":boot,"role":"update",
                    "committing":true,"owner":store::current_owner()}),
        )
        .unwrap();
        s
    }
    /// A `WIN11_UPDATE_*` seam re-roots the guard's files (GW1's bus layer): this test is for the real paths.
    fn seam_set() -> Option<String> {
        std::env::vars_os().map(|(k, _)| k.to_string_lossy().into_owned()).find(|k| k.starts_with("WIN11_UPDATE_"))
    }
    /// GE0b's sandbox (tests/rust-sandbox/run-tests.sh) and nothing else (review m7): its logging guard-bin first on
    /// PATH, every program `action` could start for reboot/shutdown/suspend/logout (`systemctl`,
    /// `gnome-session-quit`, `loginctl`) resolving into it, the system bus its socket that does not exist, and no
    /// Hyprland (`hyprctl` has no stub). Only then is `action` itself called, and only once the guard refused, so no
    /// real program can run even if the guard were broken.
    fn in_unit_sandbox() -> Result<(), String> {
        let path = std::env::var("PATH").unwrap_or_default();
        let guard_bin = path.split(':').next().filter(|p| p.ends_with("/guard-bin")).ok_or("guard-bin is not first on PATH")?;
        for name in ["systemctl", "gnome-session-quit", "loginctl"] {
            let found = path.split(':').map(|dir| std::path::Path::new(dir).join(name)).find(|p| p.is_file());
            if found.as_deref().and_then(std::path::Path::parent) != Some(std::path::Path::new(guard_bin)) {
                return Err(format!("{name} resolves to {found:?}, not into {guard_bin}"));
            }
        }
        let sandbox = std::path::Path::new(guard_bin).parent().ok_or("guard-bin has no parent")?;
        let socket = sandbox.join("run/no-system-bus");
        let system = std::env::var("DBUS_SYSTEM_BUS_ADDRESS").unwrap_or_default();
        if system != format!("unix:path={}", socket.display()) || std::fs::symlink_metadata(&socket).is_ok() {
            return Err(format!("DBUS_SYSTEM_BUS_ADDRESS is {system:?}, not the sandbox's missing socket"));
        }
        if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
            return Err("Hyprland is set".into());
        }
        Ok(())
    }

    #[test]
    fn power_actions_are_refused_while_our_update_job_commits_and_lock_passes() {
        if let Some(seam) = seam_set() {
            eprintln!("skipped: {seam} is set (a seam run re-roots the guard's files; this test checks the real paths)");
            return;
        }
        let sandbox = in_unit_sandbox();
        if let Err(why) = &sandbox {
            eprintln!("action() itself is not called: {why}");
        }
        for (lang, text) in [
            ("en", "Updates are being installed. You can restart as soon as that’s done."),
            ("nl", "Er worden updates geïnstalleerd. Opnieuw opstarten kan zodra dat klaar is."),
        ] {
            let c = LangContext::new(lang, "panel");
            let jobs = committing_job(&c.ctx);
            for name in ["reboot", "shutdown", "suspend", "logout"] {
                let refused = guard::power_blocked(&c.ctx, name).expect("the guard refuses");
                assert_eq!(refused.key.as_deref(), Some("update.guard.power"), "{name}");
                if sandbox.is_ok() {
                    let error = action(name, &c.ctx).unwrap_err();
                    assert_eq!((error.code.as_str(), error.message.as_str()), ("update_busy", text), "{lang} {name}");
                    assert_eq!(error.key.as_deref(), Some("update.guard.power"));
                }
            }
            // lock is never refused (the guard passes it; action would lock the session, so it is not called here)
            assert!(guard::power_blocked(&c.ctx, "lock").is_none());
            // an unknown action passes the guard and is refused by action itself
            assert_eq!(action("bogus", &c.ctx).unwrap_err().key.as_deref(), Some("system.power.unknownAction"));
            // a finished job no longer blocks
            let mut finished = jobs.read(store::JOB).unwrap();
            finished["finishedAt"] = json!(store::now_ms());
            jobs.write(store::JOB, &finished).unwrap();
            assert!(guard::power_blocked(&c.ctx, "reboot").is_none());
        }
    }
}
