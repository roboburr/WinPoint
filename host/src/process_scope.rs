//! Start new processes through the user manager before they can inherit a shell
//! cgroup. Only our newly created shell slice receives a memory limit.
use glib::variant::ToVariant;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const MANAGER: &str = "org.freedesktop.systemd1";
const MANAGER_PATH: &str = "/org/freedesktop/systemd1";
const MANAGER_IFACE: &str = "org.freedesktop.systemd1.Manager";
const SLICE: &str = "app-win11shell.slice";
const DESCRIPTION: &str = "Win11 GNOME shell (bounded)";
pub const MEMORY_MAX: u64 = 1000 * 1024 * 1024;
pub const MEMORY_HIGH: u64 = 768 * 1024 * 1024;
static NEXT: AtomicU64 = AtomicU64::new(0);

#[derive(Copy, Clone, Debug)]
pub enum Role {
    Shell,
    App,
}
/// The shell role: `win11-panel --http-only`, win11-shell's backend (win11-panel itself execs win11-shell before
/// this runs, lib.rs native_shell_switch).
fn shell(kind: &str) -> bool {
    kind == "panel"
}
fn managed(cgroup: &str, role: Role) -> bool {
    let own = cgroup.lines().any(|line| {
        line.split('/')
            .any(|part| part.starts_with("app-win11-") && part.ends_with(".service"))
    });
    own && match role {
        Role::Shell => cgroup.split('/').any(|part| part == SLICE),
        Role::App => !cgroup.split('/').any(|part| part == SLICE),
    }
}
fn manager() -> Result<Option<gio::DBusConnection>, String> {
    let connection = match gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };
    let reply = connection
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameHasOwner",
            Some(&(MANAGER,).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            1000,
            gio::Cancellable::NONE,
        )
        .map_err(|e| e.to_string())?;
    if reply.get::<(bool,)>().is_some_and(|(present,)| present) {
        Ok(Some(connection))
    } else {
        Ok(None)
    }
}
fn call(
    connection: &gio::DBusConnection,
    method: &str,
    params: &glib::Variant,
) -> Result<glib::Variant, String> {
    connection
        .call_sync(
            Some(MANAGER),
            MANAGER_PATH,
            MANAGER_IFACE,
            method,
            Some(params),
            None,
            gio::DBusCallFlags::NONE,
            5000,
            gio::Cancellable::NONE,
        )
        .map_err(|error| error.to_string())
}
fn slice_properties() -> Vec<(String, glib::Variant)> {
    vec![
        ("Description".into(), DESCRIPTION.to_variant()),
        ("MemoryAccounting".into(), true.to_variant()),
        ("MemoryHigh".into(), MEMORY_HIGH.to_variant()),
        ("MemoryMax".into(), MEMORY_MAX.to_variant()),
        ("CollectMode".into(), "inactive-or-failed".to_variant()),
    ]
}
fn ensure_slice(connection: &gio::DBusConnection) -> Result<(), String> {
    let aux: Vec<(String, Vec<(String, glib::Variant)>)> = Vec::new();
    if call(
        connection,
        "StartTransientUnit",
        &(SLICE, "fail", slice_properties(), aux).to_variant(),
    )
    .is_ok()
    {
        return Ok(());
    }
    // A prior launch may already own the slice. Verify identity before touching
    // an existing unit; never modify the original GNOME Panel scope or its apps.
    let unit = call(connection, "GetUnit", &(SLICE,).to_variant())?
        .get::<(glib::variant::ObjectPath,)>()
        .ok_or("Ongeldig systemd-unitadres")?
        .0;
    let properties = connection
        .call_sync(
            Some(MANAGER),
            unit.as_str(),
            "org.freedesktop.DBus.Properties",
            "GetAll",
            Some(&("org.freedesktop.systemd1.Unit",).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            3000,
            gio::Cancellable::NONE,
        )
        .map_err(|e| e.to_string())?
        .get::<(BTreeMap<String, glib::Variant>,)>()
        .ok_or("Ongeldige systemd-eigenschappen")?
        .0;
    if properties
        .get("Description")
        .and_then(|v| v.get::<String>())
        .as_deref()
        != Some(DESCRIPTION)
        || properties.get("Transient").and_then(|v| v.get::<bool>()) != Some(true)
    {
        return Err("De shell-geheugengroep bestaat al en is niet van deze installatie.".into());
    }
    call(
        connection,
        "SetUnitProperties",
        &(
            SLICE,
            true,
            slice_properties()
                .into_iter()
                .filter(|(key, _)| key != "Description" && key != "CollectMode")
                .collect::<Vec<_>>(),
        )
            .to_variant(),
    )?;
    Ok(())
}
fn environment(
    command: &Command,
    complete: Option<&[(OsString, OsString)]>,
    lang: crate::i18n::Lang,
) -> Result<Vec<String>, String> {
    let mut env: BTreeMap<OsString, OsString> = complete
        .map(|entries| entries.iter().cloned().collect())
        .unwrap_or_else(|| std::env::vars_os().collect());
    for (key, value) in command.get_envs() {
        if let Some(value) = value {
            env.insert(key.into(), value.into());
        } else {
            env.remove(key);
        }
    }
    env.into_iter()
        .map(|(key, value)| {
            Ok(format!(
                "{}={}",
                key.into_string().map_err(|_| crate::i18n::tr(lang, "launch.nonUtf8EnvName"))?,
                value
                    .into_string()
                    .map_err(|_| crate::i18n::tr(lang, "launch.nonUtf8EnvValue"))?
            ))
        })
        .collect()
}
fn manager_environment(
    connection: &gio::DBusConnection,
    lang: crate::i18n::Lang,
) -> Result<Vec<String>, String> {
    let reply = connection
        .call_sync(
            Some(MANAGER),
            MANAGER_PATH,
            "org.freedesktop.DBus.Properties",
            "Get",
            Some(&(MANAGER_IFACE, "Environment").to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            3000,
            gio::Cancellable::NONE,
        )
        .map_err(|e| e.to_string())?;
    reply
        .get::<(glib::Variant,)>()
        .and_then(|(value,)| value.get::<Vec<String>>())
        .ok_or_else(|| crate::i18n::tr(lang, "launch.invalidSystemdEnvironment"))
}
fn start_unit(
    command: &Command,
    role: Role,
    tag: &str,
    env: Option<&[(OsString, OsString)]>,
    argv0: Option<&std::ffi::OsStr>,
    lang: crate::i18n::Lang,
) -> Result<Option<(gio::DBusConnection, String)>, String> {
    let Some(connection) = manager()? else {
        return Ok(None);
    };
    if matches!(role, Role::Shell) {
        ensure_slice(&connection)?;
    }
    let program = PathBuf::from(command.get_program());
    let program = if program.is_absolute() {
        program
    } else {
        glib::find_program_in_path(program).ok_or_else(|| crate::i18n::tr(lang, "launch.programNotFound"))?
    };
    let program = program
        .into_os_string()
        .into_string()
        .map_err(|_| crate::i18n::tr(lang, "launch.nonUtf8ProgramPath"))?;
    let mut argv = vec![
        argv0
            .map(|s| s.to_str().ok_or_else(|| crate::i18n::tr(lang, "launch.nonUtf8Argument")))
            .transpose()?
            .unwrap_or(&program)
            .to_owned(),
    ];
    for arg in command.get_args() {
        argv.push(
            arg.to_str()
                .ok_or_else(|| crate::i18n::tr(lang, "launch.nonUtf8Argument"))?
                .to_owned(),
        );
    }
    let env = environment(command, env, lang)?;
    let unset: Vec<String> = manager_environment(&connection, lang)?
        .into_iter()
        .filter_map(|entry| {
            let (key, _) = entry.split_once('=')?;
            (!env
                .iter()
                .any(|wanted| wanted.split_once('=').is_some_and(|(name, _)| name == key)))
            .then(|| key.to_owned())
        })
        .collect();
    let tag: String = tag
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(32)
        .collect();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let unit = format!(
        "app-win11-{tag}-{}-{nonce}-{}.service",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let mut properties: Vec<(String, glib::Variant)> = vec![
        (
            "Description".into(),
            format!("Win11 GNOME {tag}").to_variant(),
        ),
        ("Type".into(), "exec".to_variant()),
        ("ExitType".into(), "cgroup".to_variant()),
        ("CollectMode".into(), "inactive-or-failed".to_variant()),
        (
            "Slice".into(),
            match role {
                Role::Shell => SLICE,
                Role::App => "app.slice",
            }
            .to_variant(),
        ),
        (
            "ExecStartEx".into(),
            vec![(program, argv, vec!["no-env-expand".to_owned()])].to_variant(),
        ),
        ("Environment".into(), env.to_variant()),
        ("UnsetEnvironment".into(), unset.to_variant()),
        ("StandardInput".into(), "null".to_variant()),
        ("StandardOutput".into(), "journal".to_variant()),
        ("StandardError".into(), "journal".to_variant()),
        // D-G66: a service outlives the GNOME session unless it is part of it. Robin, 13-11: "ik kan zen browser
        // niet openen": Zen, launched by us before a logout, ran on without a display and took every new launch.
        (
            "PartOf".into(),
            vec!["graphical-session.target".to_owned()].to_variant(),
        ),
    ];
    let cwd = command
        .get_current_dir()
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok());
    if let Some(path) = cwd {
        properties.push((
            "WorkingDirectory".into(),
            path.to_str().ok_or_else(|| crate::i18n::tr(lang, "launch.nonUtf8WorkingDirectory"))?.to_variant(),
        ));
    }
    let aux: Vec<(String, Vec<(String, glib::Variant)>)> = Vec::new();
    call(
        &connection,
        "StartTransientUnit",
        &(unit.clone(), "fail", properties, aux).to_variant(),
    )?;
    Ok(Some((connection, unit)))
}
fn start(
    command: &Command,
    role: Role,
    tag: &str,
    env: Option<&[(OsString, OsString)]>,
) -> Result<bool, String> {
    let lang = crate::i18n::current_language();
    Ok(start_unit(command, role, tag, env, None, lang)?.is_some())
}
/// New native entry points relocate before they start a backend, a worker or a GUI.
pub fn ensure_native(kind: &str) -> Result<bool, String> {
    let role = if shell(kind) { Role::Shell } else { Role::App };
    if fs::read_to_string("/proc/self/cgroup").is_ok_and(|group| managed(&group, role)) {
        return Ok(false);
    }
    let mut command = Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
    command.args(std::env::args_os().skip(1));
    if let Ok(dir) = std::env::current_dir() {
        command.current_dir(dir);
    }
    let started = start(&command, role, kind, None)?;
    if !started {
        eprintln!(
            "Win11 GNOME: geen systemd-gebruikersbeheerder; procesisolatie en de shell-geheugenlimiet zijn hier niet beschikbaar."
        );
    }
    Ok(started)
}
/// Spawn an independent app. The optional complete environment preserves the
/// existing file-launch sanitation rather than inheriting host-only variables.
pub fn spawn(
    mut command: Command,
    tag: &str,
    env: Option<Vec<(OsString, OsString)>>,
) -> Result<(), String> {
    crate::launch_context::configure_portal_child_environment(&mut command, env.as_deref());
    if start(&command, Role::App, tag, env.as_deref())? {
        return Ok(());
    }
    eprintln!(
        "Win11 GNOME: geen systemd-gebruikersbeheerder; app wordt zonder cgroup-isolatie gestart."
    );
    if let Some(env) = env {
        let merged = environment(&command, Some(&env), crate::i18n::current_language())?;
        let merged: Vec<(OsString, OsString)> = merged
            .into_iter()
            .filter_map(|entry| {
                entry
                    .split_once('=')
                    .map(|(key, value)| (key.into(), value.into()))
            })
            .collect();
        command.env_clear().envs(merged);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Launch a restarted task outside the shell slice and report its actual process.
pub(crate) fn spawn_pid(
    command: Command,
    tag: &str,
    env: Option<Vec<(OsString, OsString)>>,
) -> Result<u32, String> {
    spawn_pid_with_argv0(command, tag, env, None)
}
pub(crate) fn spawn_pid_with_argv0(
    mut command: Command,
    tag: &str,
    env: Option<Vec<(OsString, OsString)>>,
    argv0: Option<OsString>,
) -> Result<u32, String> {
    crate::launch_context::configure_portal_child_environment(&mut command, env.as_deref());
    use std::os::unix::process::CommandExt;
    let lang = crate::i18n::current_language();
    if let Some((connection, unit)) =
        start_unit(&command, Role::App, tag, env.as_deref(), argv0.as_deref(), lang)?
    {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        loop {
            let unit_path = call(&connection, "GetUnit", &(unit.as_str(),).to_variant())?
                .get::<(glib::variant::ObjectPath,)>()
                .ok_or_else(|| crate::i18n::tr(lang, "launch.invalidServicePath"))?
                .0;
            let result = connection
                .call_sync(
                    Some(MANAGER),
                    unit_path.as_str(),
                    "org.freedesktop.DBus.Properties",
                    "GetAll",
                    Some(&("org.freedesktop.systemd1.Service",).to_variant()),
                    None,
                    gio::DBusCallFlags::NONE,
                    2000,
                    gio::Cancellable::NONE,
                )
                .map_err(|e| e.to_string())?;
            let values = result
                .get::<(BTreeMap<String, glib::Variant>,)>()
                .ok_or_else(|| crate::i18n::tr(lang, "launch.invalidServiceState"))?
                .0;
            for key in ["MainPID", "ExecMainPID"] {
                if let Some(pid) = values
                    .get(key)
                    .and_then(|v| v.get::<u32>())
                    .filter(|p| *p > 0)
                {
                    return Ok(pid);
                }
            }
            if values
                .get("Result")
                .and_then(|v| v.str())
                .is_some_and(|v| v != "success")
            {
                return Err(crate::i18n::tr(lang, "launch.programFailed").into());
            }
            if std::time::Instant::now() >= deadline {
                return Err(crate::i18n::tr(lang, "launch.programTimedOut").into());
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    if let Some(env) = env {
        let merged = environment(&command, Some(&env), lang)?;
        command.env_clear();
        for entry in merged {
            if let Some((key, value)) = entry.split_once('=') {
                command.env(key, value);
            }
        }
    }
    if let Some(argv0) = argv0 {
        command.arg0(argv0);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let pid = child.id();
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(pid)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_own_correct_slice_counts_as_managed() {
        assert!(managed(
            "0::/user.slice/app.slice/app-win11shell.slice/app-win11-panel-1.service",
            Role::Shell
        ));
        assert!(!managed(
            "0::/user.slice/app.slice/app-gnome-Panel.scope",
            Role::Shell
        ));
        assert!(!managed(
            "0::/app.slice/app-win11-external-1.service",
            Role::Shell
        ));
        assert!(managed(
            "0::/app.slice/app-win11-settings-1.service",
            Role::App
        ));
        assert!(!managed(
            "0::/app.slice/app-win11shell.slice/app-win11-panel-1.service",
            Role::App
        ));
    }
    #[test]
    fn aggregate_cap_is_below_one_gib_and_high_below_max() {
        assert!(MEMORY_MAX < 1024 * 1024 * 1024);
        assert!(MEMORY_HIGH < MEMORY_MAX);
    }
}

#[cfg(test)]
mod manager_tests {
    use super::*;
    #[test]
    fn explicit_clean_environment_preserves_fresh_command_overrides() {
        let mut command = Command::new("/usr/bin/true");
        command
            .env("XDG_ACTIVATION_TOKEN", "fresh-token")
            .env_remove("DESKTOP_STARTUP_ID");
        let base = vec![
            ("PATH".into(), "/usr/bin".into()),
            ("DESKTOP_STARTUP_ID".into(), "stale-token".into()),
        ];
        let values = environment(&command, Some(&base), crate::i18n::Lang::Nl).unwrap();
        assert!(values.contains(&"XDG_ACTIVATION_TOKEN=fresh-token".into()));
        assert!(values.contains(&"PATH=/usr/bin".into()));
        assert!(
            !values
                .iter()
                .any(|value| value.starts_with("DESKTOP_STARTUP_ID="))
        );
        assert_eq!(values.len(), 2);
    }
    #[test]
    fn a_non_utf8_environment_is_refused_in_the_settings_language() {
        // launch stem (round2-gate §6): environment() builds the unit's Environment= list before any D-Bus call
        use std::os::unix::ffi::OsStringExt;
        let command = Command::new("/usr/bin/true");
        let name = vec![(OsString::from_vec(vec![b'X', 0xff]), OsString::from("1"))];
        let value = vec![(OsString::from("X"), OsString::from_vec(vec![0xff]))];
        let refused =
            |base: &[(OsString, OsString)], lang| environment(&command, Some(base), lang).unwrap_err();
        assert_eq!(refused(&name, crate::i18n::Lang::Nl), "Niet-UTF8 omgevingsnaam");
        assert_eq!(refused(&name, crate::i18n::Lang::En), "Non-UTF-8 environment variable name");
        assert_eq!(refused(&value, crate::i18n::Lang::Nl), "Niet-UTF8 omgevingswaarde");
        assert_eq!(refused(&value, crate::i18n::Lang::En), "Non-UTF-8 environment variable value");
    }
    #[test]
    #[ignore = "Creates only short-lived own Python probe services on the actual user manager"]
    fn actual_manager_separates_app_from_bounded_shell_slice() {
        let Some(_connection) = manager().unwrap() else {
            panic!("No real user manager")
        };
        let root = std::env::temp_dir().join(format!(
            "win11-cgroup-proof-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let code = r#"import json,os,pathlib,sys,time
p=pathlib.Path(sys.argv[1]);g=pathlib.Path('/proc/self/cgroup').read_text().strip().split('::',1)[1];parent=pathlib.Path('/sys/fs/cgroup'+g).parent
p.write_text(json.dumps({'cgroup':g,'parent_memory_max':(parent/'memory.max').read_text().strip(),'parent_memory_high':(parent/'memory.high').read_text().strip(),'cwd':os.getcwd(),'token':os.environ.get('XDG_ACTIVATION_TOKEN'),'stale':os.environ.get('DESKTOP_STARTUP_ID'),'literal':sys.argv[2]}))
time.sleep(2)
"#;
        let literal = "quote ' \" $(touch SHOULDNTEVALUATE) `no-shell` ${HOME} $HOME %n %u %% %h {\"path\":\"/tmp/${HOME}/%u\"}";
        for (role, name) in [(Role::Shell, "shell"), (Role::App, "app")] {
            let output = root.join(format!("{name}.json"));
            let mut command = Command::new("/usr/bin/python3");
            command
                .arg("-c")
                .arg(code)
                .arg(&output)
                .arg(literal)
                .current_dir(&root)
                .env("XDG_ACTIVATION_TOKEN", "fresh-probe-token")
                .env_remove("DESKTOP_STARTUP_ID");
            let env = vec![
                ("PATH".into(), "/usr/bin".into()),
                ("HOME".into(), root.as_os_str().to_owned()),
                ("DESKTOP_STARTUP_ID".into(), "stale-token".into()),
            ];
            assert!(start(&command, role, &format!("probe-{name}"), Some(&env)).unwrap());
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
            while !output.is_file() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "Probe did not execute"
                );
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
            assert_eq!(value["token"], "fresh-probe-token");
            assert!(value["stale"].is_null());
            assert_eq!(value["literal"], literal);
            assert_eq!(value["cwd"], root.to_str().unwrap());
            let cgroup = value["cgroup"].as_str().unwrap();
            assert!(managed(cgroup, role));
            if matches!(role, Role::Shell) {
                assert_eq!(value["parent_memory_max"], MEMORY_MAX.to_string());
                assert_eq!(value["parent_memory_high"], MEMORY_HIGH.to_string());
            } else {
                assert!(!cgroup.contains(SLICE));
            }
        }
        assert!(!root.join("SHOULDNTEVALUATE").exists());
        println!("Cgroup probe evidence: {}", root.display());
        // These processes finish themselves; no existing user unit is stopped.
        std::thread::sleep(std::time::Duration::from_secs(3));
    }
}
