//! The trusted-helper check and the pkexec runner (design §6.3, §6.4 step 6, §7; the pattern of
//! `services/storage/cleanup.rs` `admin_helper`/`run_admin`), GW5.
//!
//! * A helper counts only when it is a regular file (not a symlink) owned by root with no group/other write, in a
//!   directory owned by root with no group/other write. Test mode trusts the stub directory's owner instead (the
//!   euid): every program there is a stub of the marked tool directory anyway (`paths.rs`).
//! * `update.release.setPrompt`: `pkexec <release-upgrades-helper> set-prompt <v>` (polkit action
//!   `io.github.win11shell.gnome.release-upgrades`, auth_admin_keep). Without a trusted helper, the same script
//!   text ([`PROMPT_SCRIPT`], the helper file itself) through `pkexec /bin/sh -c '<script>' win11-gnome-prompt
//!   set-prompt <v>`, and the page shows `promptShInfo` (snapshot `release.helper.prompt` false). pkexec and `sh`
//!   come from the tool table only. pkexec 126 (the dialog was dismissed) → `update.release.err.noPassword`, 127
//!   (not authorised, a wrong password) → `.wrongPassword`, both only when no `@@` marker was printed; any other
//!   end without `@@ok prompt=<v>` → `.promptFailed`. A cleared environment (the table's `PATH` and `LANG`), no
//!   stdin, at most 10 minutes (the password dialog waits for the user).
//! * `update.release.attach` (GW7 runs it in the terminal wrapper): [`attach_argv`] is `pkexec
//!   <release-upgrade-attach>` (auth_admin, never kept), or `update.release.err.noAttachHelper` with the manual
//!   command; our code never types sudo.
use super::{
    Shared,
    paths::{Setup, Tool},
};
use crate::{
    files::{ApiError, ApiResult},
    i18n::{Lang, NO_PARAMS},
};
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::{Child, Stdio},
    time::{Duration, Instant},
};

/// The Prompt helper's own text: the `/bin/sh -c` fallback runs exactly the tested script (design §6.3).
pub(crate) const PROMPT_SCRIPT: &str = include_str!("../../../data/release-upgrades-helper");
/// `$0` of the fallback (`sh -c <script> win11-gnome-prompt set-prompt <v>`).
pub(crate) const FALLBACK_NAME: &str = "win11-gnome-prompt";
const TIMEOUT: Duration = Duration::from_secs(600);
const MAX_OUTPUT: u64 = 64 << 10;

/// A regular file of `owner` without group/other write, in a directory of `owner` without group/other write.
pub(crate) fn trusted_file(path: &Path, owner: u32) -> bool {
    let Ok(file) = std::fs::symlink_metadata(path) else {
        return false;
    };
    let Some(parent) = path.parent().and_then(|p| std::fs::metadata(p).ok()) else {
        return false;
    };
    file.is_file()
        && file.uid() == owner
        && file.mode() & 0o022 == 0
        && parent.uid() == owner
        && parent.mode() & 0o022 == 0
}
/// The helper's path when it is trusted (production: root's file; test mode: the stub directory's owner).
pub(super) fn trusted_helper(setup: &Setup, tool: Tool) -> Option<PathBuf> {
    let owner = if setup.test {
        unsafe { libc::geteuid() }
    } else {
        0
    };
    setup
        .tools
        .path(tool)
        .ok()
        .filter(|path| trusted_file(path, owner))
}
/// The snapshot's `release.helper` (design §4.3): which trusted helpers are installed.
pub(super) fn helpers(setup: &Setup) -> Value {
    json!({"prompt":trusted_helper(setup, Tool::PromptHelper).is_some(),
           "attach":trusted_helper(setup, Tool::AttachHelper).is_some()})
}

/// The pkexec arguments of a Prompt change and whether they are the `/bin/sh` fallback.
pub(super) fn prompt_argv(setup: &Setup, args: &[&str]) -> Option<(Vec<OsString>, bool)> {
    if let Some(helper) = trusted_helper(setup, Tool::PromptHelper) {
        let mut argv = vec![helper.into_os_string()];
        argv.extend(args.iter().map(OsString::from));
        return Some((argv, false));
    }
    let sh = setup.tools.path(Tool::Sh).ok()?;
    let mut argv = vec![
        sh.into_os_string(),
        "-c".into(),
        PROMPT_SCRIPT.into(),
        FALLBACK_NAME.into(),
    ];
    argv.extend(args.iter().map(OsString::from));
    Some((argv, true))
}

/// What one run printed: the `@@` marker lines (in order) and the exit code (None: killed, or no code).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Run {
    pub rc: Option<i32>,
    pub markers: Vec<String>,
}
/// The §3.7 key of a run that did not succeed (None: `want` was printed, no `@@fail`, exit 0).
pub(crate) fn failure_key(run: &Run, want: &str) -> Option<&'static str> {
    if run.markers.is_empty() {
        match run.rc {
            Some(126) => return Some("update.release.err.noPassword"),
            Some(127) => return Some("update.release.err.wrongPassword"),
            _ => {}
        }
    }
    let ok = run.markers.iter().any(|m| m == want);
    let failed = run.markers.iter().any(|m| m.starts_with("@@fail"));
    (!ok || failed || run.rc != Some(0)).then_some("update.release.err.promptFailed")
}
fn error(key: &str, lang: Lang) -> ApiError {
    let code = match key {
        "update.release.err.noPassword" => "auth_cancelled",
        "update.release.err.wrongPassword" => "auth_failed",
        _ => "failed",
    };
    ApiError::tr(code, lang, key, NO_PARAMS)
}

/// `pkexec <helper> set-prompt <v>` (or the `/bin/sh` fallback) through the tool table: `{markers, fallback}`
/// when `@@ok prompt=<v>` came, else the §3.7 error (module doc). Only the Prompt helper runs this way; the attach
/// helper runs in a terminal (GW7, [`attach_argv`]).
pub(super) fn run(shared: &Shared, helper: Tool, args: &[&str], lang: Lang) -> ApiResult {
    let failed = || error("update.release.err.promptFailed", lang);
    if helper != Tool::PromptHelper || args.len() != 2 || args[0] != "set-prompt" {
        return Err(failed());
    }
    let want = format!("@@ok prompt={}", args[1]);
    let setup = &shared.setup;
    let Some((argv, fallback)) = prompt_argv(setup, args) else {
        return Err(failed());
    };
    let Ok(mut command) = setup.tools.command(Tool::Pkexec) else {
        return Err(failed());
    };
    command
        .args(&argv)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let Some(run) = command.spawn().ok().and_then(|child| wait(child, TIMEOUT)) else {
        return Err(failed());
    };
    match failure_key(&run, &want) {
        Some(key) => Err(error(key, lang)),
        None => Ok(json!({"markers":run.markers,"fallback":fallback})),
    }
}
/// Read at most [`MAX_OUTPUT`] of a child's stdout (when piped) and wait at most `timeout`, then kill it.
pub(super) fn wait(mut child: Child, timeout: Duration) -> Option<Run> {
    let reader = child.stdout.take().map(|mut stdout| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = (&mut stdout).take(MAX_OUTPUT).read_to_end(&mut bytes);
            bytes
        })
    });
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
    let out = reader
        .map(|r| r.join().unwrap_or_default())
        .unwrap_or_default();
    let markers = String::from_utf8_lossy(&out)
        .lines()
        .map(str::trim_end)
        .filter(|l| l.starts_with("@@"))
        .map(str::to_owned)
        .collect();
    Some(Run { rc, markers })
}

/// GW7's attach command: `[pkexec, release-upgrade-attach]` from the tool table, or the manual command
/// (`update.release.err.noAttachHelper`) when the helper is missing or not trusted.
#[cfg_attr(not(test), allow(dead_code))] // GW7 (upgrade.rs `attach`)
pub(super) fn attach_argv(shared: &Shared, lang: Lang) -> Result<Vec<PathBuf>, ApiError> {
    let setup = &shared.setup;
    match (
        setup.tools.path(Tool::Pkexec),
        trusted_helper(setup, Tool::AttachHelper),
    ) {
        (Ok(pkexec), Some(helper)) => Ok(vec![pkexec, helper]),
        _ => Err(ApiError::tr(
            "no_helper",
            lang,
            "update.release.err.noAttachHelper",
            NO_PARAMS,
        )),
    }
}

// ------------------------------------------------------------------------------------------------ tests

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::services::update::{
        Shared, Stats, paths,
        paths::{Env, VAR_FIXTURE_ROOT, VAR_TOOL_DIR},
        store,
        tests::{Fixture, script, write},
    };
    use std::{fs, os::unix::fs::PermissionsExt, sync::atomic::AtomicU64};

    /// A file of tests/update/fixtures/release (read at run time: the source tarball has no tests/).
    pub(crate) fn fixture(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/update/fixtures/release")
            .join(name);
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }
    pub(crate) fn default_file() -> String {
        fixture("etc/update-manager/release-upgrades")
    }
    pub(crate) fn shared(f: &Fixture, lang: &str) -> Shared {
        let ctx = f.ctx("settings", false, lang);
        let (tools, root) = (f.tools.display().to_string(), f.root.display().to_string());
        let env = Env::fixture(
            &[
                (VAR_TOOL_DIR, tools.as_str()),
                (VAR_FIXTURE_ROOT, root.as_str()),
            ],
            unsafe { libc::geteuid() },
        );
        let store = store::Store::open(ctx.cache_dir.join("update")).unwrap();
        wire_settings(&ctx);
        Shared {
            ctx,
            setup: paths::resolve(&env).unwrap(),
            store,
            stats: Stats::default(),
            rev: AtomicU64::new(0),
        }
    }
    /// `settings.set` through a settings store on the context's own directory (as options.rs's tests do).
    pub(crate) fn wire_settings(ctx: &std::sync::Arc<crate::services::ServiceContext>) {
        let settings = crate::settings::SettingsStore::new(ctx.config_dir.clone()).unwrap();
        ctx.set_caller(std::sync::Arc::new(move |method, args| {
            match settings.call(method, args) {
                Some(result) => result.map(|(value, _)| value),
                None => Err(ApiError::new("unknown_method", method)),
            }
        }));
    }
    /// A pkexec stub: writes its argv (NUL-separated) to `pkexec.argv`, then does what `mode` says: an exit code,
    /// or `answer` (runs its arguments as the user, with WIN11_HELPER_ETC = the fixture's etc). Builtins only: its
    /// PATH is the stub directory alone. The fallback's `sh` stub is the real shell.
    pub(crate) fn pkexec_stub(f: &Fixture, mode: &str) {
        fs::write(f.base.join("pkexec.mode"), format!("{mode}\n")).unwrap();
        script(
            &f.tools.join("pkexec"),
            &format!(
                "echo \"pkexec $1\" >> '{log}'\nfor a in \"$@\"; do printf '%s\\0' \"$a\"; done > '{argv}'\n\
                 read -r mode < '{mode}'\ncase \"$mode\" in\n  answer) WIN11_HELPER_ETC='{etc}'; export WIN11_HELPER_ETC; exec \"$@\" ;;\n  \
                 *) exit \"$mode\" ;;\nesac\n",
                log = f.log.display(),
                argv = f.base.join("pkexec.argv").display(),
                mode = f.base.join("pkexec.mode").display(),
                etc = f.root.join("etc").display(),
            ),
        );
        script(&f.tools.join("sh"), "exec /bin/sh \"$@\"\n");
    }
    pub(crate) fn pkexec_argv(f: &Fixture) -> Vec<String> {
        let raw = fs::read(f.base.join("pkexec.argv")).unwrap_or_default();
        raw.split(|b| *b == 0)
            .filter(|a| !a.is_empty())
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .collect()
    }
    /// The real helper as the tool table's PromptHelper (the stub directory's owner: trusted in test mode).
    pub(crate) fn install_helper(f: &Fixture) {
        let path = f.tools.join("release-upgrades-helper");
        let _ = fs::remove_file(&path);
        fs::write(&path, PROMPT_SCRIPT).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    pub(crate) fn release_file(f: &Fixture) -> std::path::PathBuf {
        f.root.join("etc/update-manager/release-upgrades")
    }

    #[test]
    fn the_trust_rule_is_cleanup_rs_rule() {
        let f = Fixture::new("admin-trust");
        let me = unsafe { libc::geteuid() };
        let helper = f.tools.join("release-upgrades-helper");
        assert!(
            trusted_file(&helper, me),
            "the fixture's stub: ours, 0755, in our 0700 directory"
        );
        assert!(!trusted_file(&helper, 0), "not root's");
        // root's file in root's directory (read only; skipped inside a user namespace that maps root to nobody, as
        // tests/update/run.sh's layer does)
        if std::fs::metadata("/etc/passwd").is_ok_and(|m| m.uid() == 0)
            && std::fs::metadata("/etc").is_ok_and(|m| m.uid() == 0)
        {
            assert!(
                trusted_file(Path::new("/etc/passwd"), 0)
                    && !trusted_file(Path::new("/etc/passwd"), me),
                "root's file in root's directory"
            );
        }
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o775)).unwrap();
        assert!(!trusted_file(&helper, me), "group write");
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&f.tools, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(!trusted_file(&helper, me), "a writable directory");
        fs::set_permissions(&f.tools, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_file(&helper).unwrap();
        std::os::unix::fs::symlink(f.tools.join("sh"), &helper).unwrap();
        assert!(!trusted_file(&helper, me), "a symlink");
        fs::remove_file(&helper).unwrap();
        assert!(!trusted_file(&helper, me), "missing");
        // the snapshot's flags follow the same rule
        let shared = shared(&f, "nl");
        assert_eq!(
            helpers(&shared.setup),
            json!({"prompt":false,"attach":true})
        );
        assert_eq!(f.stub_log(), "", "the check runs nothing");
    }

    #[test]
    fn the_prompt_change_runs_pkexec_with_the_helper_or_the_same_script_through_sh() {
        let f = Fixture::new("admin-run");
        let default = default_file();
        write(&release_file(&f), &default);
        install_helper(&f);
        pkexec_stub(&f, "answer");
        let shared = shared(&f, "nl");
        let ok = run(
            &shared,
            Tool::PromptHelper,
            &["set-prompt", "lts"],
            Lang::Nl,
        )
        .unwrap();
        assert_eq!(ok, json!({"markers":["@@ok prompt=lts"],"fallback":false}));
        let helper = f
            .tools
            .join("release-upgrades-helper")
            .display()
            .to_string();
        assert_eq!(
            pkexec_argv(&f),
            [helper.clone(), "set-prompt".into(), "lts".into()]
        );
        assert_eq!(
            fs::read_to_string(release_file(&f)).unwrap(),
            default.replace("Prompt=normal\n", "Prompt=lts\n"),
            "only the Prompt line"
        );
        // no trusted helper: pkexec /bin/sh -c <the helper's text> win11-gnome-prompt set-prompt normal
        fs::remove_file(f.tools.join("release-upgrades-helper")).unwrap();
        let ok = run(
            &shared,
            Tool::PromptHelper,
            &["set-prompt", "normal"],
            Lang::Nl,
        )
        .unwrap();
        assert_eq!(ok["fallback"], true);
        let sh = f.tools.join("sh").display().to_string();
        let argv = pkexec_argv(&f);
        assert_eq!(
            argv,
            [
                sh.as_str(),
                "-c",
                PROMPT_SCRIPT,
                FALLBACK_NAME,
                "set-prompt",
                "normal"
            ]
        );
        assert_eq!(
            fs::read_to_string(release_file(&f)).unwrap(),
            default,
            "back to Ubuntu's file, byte for byte"
        );
        // an untrusted helper is not used either
        install_helper(&f);
        fs::set_permissions(
            f.tools.join("release-upgrades-helper"),
            fs::Permissions::from_mode(0o777),
        )
        .unwrap();
        assert_eq!(
            run(
                &shared,
                Tool::PromptHelper,
                &["set-prompt", "never"],
                Lang::Nl
            )
            .unwrap()["fallback"],
            true
        );
        assert!(
            fs::read_to_string(release_file(&f))
                .unwrap()
                .contains("\nPrompt=never\n")
        );
        assert_eq!(
            f.stub_log(),
            format!("pkexec {helper}\npkexec {sh}\npkexec {sh}\n")
        );
        assert_eq!(
            shared.setup.tools.runs(),
            3,
            "one pkexec per change, nothing else"
        );
    }

    #[test]
    fn pkexec_126_and_127_and_failures_become_the_design_texts() {
        let f = Fixture::new("admin-rc");
        let default = default_file();
        write(&release_file(&f), &default);
        install_helper(&f);
        for (lang, mode, code, message) in [
            (
                "nl",
                "126",
                "auth_cancelled",
                "Je hebt het wachtwoord niet ingevuld, dus er is niets veranderd.",
            ),
            (
                "en",
                "126",
                "auth_cancelled",
                "You didn’t enter the password, so nothing was changed.",
            ),
            (
                "nl",
                "127",
                "auth_failed",
                "Het wachtwoord klopte niet. Er is niets veranderd.",
            ),
            (
                "en",
                "127",
                "auth_failed",
                "The password was wrong. Nothing was changed.",
            ),
            (
                "nl",
                "1",
                "failed",
                "De instelling voor nieuwe Ubuntu-versies kon niet worden opgeslagen.",
            ),
            (
                "en",
                "0",
                "failed",
                "The setting for new Ubuntu versions couldn’t be saved.",
            ),
        ] {
            pkexec_stub(&f, mode);
            let shared = shared(&f, lang);
            let error = run(
                &shared,
                Tool::PromptHelper,
                &["set-prompt", "lts"],
                shared.ctx.lang(),
            )
            .unwrap_err();
            assert_eq!(
                (error.code.as_str(), error.message.as_str()),
                (code, message),
                "{lang} {mode}"
            );
            assert_eq!(fs::read_to_string(release_file(&f)).unwrap(), default);
        }
        // 126/127 count only without a marker (the cleanup.rs rule); a marker decides otherwise
        let run_of = |rc, markers: &[&str]| Run {
            rc: Some(rc),
            markers: markers.iter().map(|m| m.to_string()).collect(),
        };
        assert_eq!(
            failure_key(&run_of(126, &["@@fail nodir"]), "@@ok prompt=lts"),
            Some("update.release.err.promptFailed")
        );
        assert_eq!(
            failure_key(
                &run_of(0, &["@@ok prompt=lts", "@@override /etc/x.cfg"]),
                "@@ok prompt=lts"
            ),
            None
        );
        assert_eq!(
            failure_key(&run_of(0, &["@@ok prompt=normal"]), "@@ok prompt=lts"),
            Some("update.release.err.promptFailed")
        );
        assert_eq!(
            failure_key(&run_of(1, &["@@ok prompt=lts"]), "@@ok prompt=lts"),
            Some("update.release.err.promptFailed")
        );
        assert_eq!(
            failure_key(
                &Run {
                    rc: None,
                    markers: vec![]
                },
                "@@ok prompt=lts"
            ),
            Some("update.release.err.promptFailed"),
            "killed"
        );
        // the helper's own refusal (no update-manager directory in the fixture etc): @@fail nodir
        fs::remove_dir_all(f.root.join("etc/update-manager")).unwrap();
        pkexec_stub(&f, "answer");
        let shared = shared(&f, "nl");
        assert_eq!(
            run(
                &shared,
                Tool::PromptHelper,
                &["set-prompt", "lts"],
                Lang::Nl
            )
            .unwrap_err()
            .key
            .as_deref(),
            Some("update.release.err.promptFailed")
        );
        // only set-prompt with one value runs at all
        let before = shared.setup.tools.runs();
        for args in [
            &["set-prompt"][..],
            &["get-prompt", "lts"],
            &["set-prompt", "lts", "x"],
        ] {
            assert!(run(&shared, Tool::PromptHelper, args, Lang::Nl).is_err());
        }
        assert!(
            run(
                &shared,
                Tool::AttachHelper,
                &["set-prompt", "lts"],
                Lang::Nl
            )
            .is_err()
        );
        assert_eq!(shared.setup.tools.runs(), before);
    }

    #[test]
    fn attach_needs_the_trusted_helper() {
        let f = Fixture::new("admin-attach");
        let shared = shared(&f, "en");
        assert_eq!(
            attach_argv(&shared, Lang::En).unwrap(),
            [
                f.tools.join("pkexec"),
                f.tools.join("release-upgrade-attach")
            ]
        );
        fs::remove_file(f.tools.join("release-upgrade-attach")).unwrap();
        let error = attach_argv(&shared, Lang::En).unwrap_err();
        assert_eq!(error.code, "no_helper");
        assert_eq!(
            error.message,
            "Open a terminal and type: sudo screen -d -r ubuntu-release-upgrade-screen-window"
        );
        assert_eq!(f.stub_log(), "");
    }
}
