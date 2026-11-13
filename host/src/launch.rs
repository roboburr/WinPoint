//! Launch each existing Win11Shell application in a separate Rust window host.
//!
//! In particular, never pass these application launches through the temporary
//! compatibility server's `serve` mode: that mode emits an in-page preview event
//! instead of creating an actual desktop application window.

use serde_json::{Value, json};
use std::process::Command;

pub const OWN_APPS: &[&str] = &[
    "explorer",
    "settings",
    "taskmgr",
    "alles",
    "disks",
    "defaultapps",
    "installer",
];

/// Construct argv directly, preserving one JSON argument without any shell
/// evaluation. Creating this command performs no application or file action.
pub fn command(app: &str, params: Option<&Value>) -> Result<Command, String> {
    command_in(app, params, crate::i18n::current_language())
}
/// `command` with its messages in `lang` (the process's settings language in production).
fn command_in(app: &str, params: Option<&Value>, lang: crate::i18n::Lang) -> Result<Command, String> {
    let params = params.filter(|value| !value.is_null());
    let binary = match app {
        "explorer" => "win11-explorer",
        "defaultapps" => "win11-default-apps",
        "installer" => "win11-installer",
        "settings" | "taskmgr" | "alles" | "disks" => "win11-app",
        _ => return Err(crate::i18n::tr(lang, "launch.unknownApp").into()),
    };
    if params.is_some_and(|value| !value.is_object()) {
        return Err(crate::i18n::tr(lang, "launch.paramsNotObject").into());
    }
    let executable = std::env::current_exe()
        .map_err(|error| crate::i18n::trf(lang, "launch.hostNotFound", &serde_json::json!({"error": error.to_string()})))?
        .with_file_name(binary);
    let mut command = Command::new(executable);
    if !matches!(app, "explorer" | "defaultapps" | "installer") {
        command.arg("--app").arg(app);
    }
    if let Some(params) = params {
        command.arg("--params").arg(params.to_string());
    }
    Ok(command)
}

/// A shell click is not inherited by a newly spawned Wayland client. Request a fresh
/// compositor startup token for this launch; never reuse the parent's consumed token.
/// GNOME remains responsible for deciding whether the eventual window may take focus.
pub fn prepare_activation(command: &mut Command, app: &str) {
    command.env_remove("DESKTOP_STARTUP_ID");
    command.env_remove("XDG_ACTIVATION_TOKEN");
    if let Some(token) = activation_token(&json!({"app":app})) {
        command.env("DESKTOP_STARTUP_ID", &token);
        command.env("XDG_ACTIVATION_TOKEN", token);
    }
}

pub fn spawn(command: Command) -> Result<(), String> {
    crate::process_scope::spawn(command, "app", None)
}

pub(crate) fn activation_token(app: &Value) -> Option<String> {
    crate::gnome::call("apps.activationToken", app)
        .ok()?
        .get("token")?
        .as_str()
        .filter(|token| !token.is_empty() && token.len() <= 4096 && !token.contains('\0'))
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn argv(command: &Command) -> Vec<String> {
        command
            .get_args()
            .map(|arg| arg.to_str().expect("generated UTF-8 argv").to_owned())
            .collect()
    }

    #[test]
    fn all_existing_apps_target_a_real_rust_window_binary() {
        for app in OWN_APPS {
            let command = command(app, None).unwrap();
            let expected = if *app == "explorer" {
                "win11-explorer"
            } else if *app == "defaultapps" {
                "win11-default-apps"
            } else if *app == "installer" {
                "win11-installer"
            } else {
                "win11-app"
            };
            assert!(command.get_program().to_string_lossy().ends_with(expected));
            if matches!(*app, "explorer" | "defaultapps" | "installer") {
                assert!(argv(&command).is_empty());
            } else {
                assert_eq!(argv(&command), ["--app", app]);
            }
        }
    }

    #[test]
    fn parameters_round_trip_as_one_literal_json_argument() {
        let params = json!({
            "path": "/tmp/Map met spaties/\"concept\"\n$(touch ongewenst)`echo nee`",
            "tabs": [{"path":"/tmp/ümlaut ' map"}],
            "select": ["bestand.txt"],
        });
        for app in OWN_APPS {
            let command = command(app, Some(&params)).unwrap();
            let args = argv(&command);
            let index = if matches!(*app, "explorer" | "defaultapps" | "installer") {
                0
            } else {
                2
            };
            assert_eq!(args[index], "--params");
            assert_eq!(args.len(), index + 2);
            assert_eq!(
                serde_json::from_str::<Value>(&args[index + 1]).unwrap(),
                params
            );
        }
    }

    #[test]
    fn unknown_apps_and_malformed_params_fail_before_any_spawn() {
        for app in ["", "panel", "settings; touch ongewenst", "../../bin/sh"] {
            assert!(command(app, None).is_err());
        }
        assert!(command("settings", Some(&json!([]))).is_err());
        assert_eq!(
            argv(&command("settings", Some(&Value::Null)).unwrap()),
            ["--app", "settings"]
        );
    }
    /// The en render test of the `launch` stem (round2-gate §6): launch::command's checks; process_scope.rs and
    /// bin/launch.rs test their own sites.
    #[test]
    fn launch_errors_follow_the_settings_language() {
        use crate::i18n::Lang;
        let error = |app: &str, params: Option<&Value>, lang| command_in(app, params, lang).unwrap_err();
        assert_eq!(error("nope", None, Lang::Nl), "Deze Win11Shell-app bestaat niet.");
        assert_eq!(error("nope", None, Lang::En), "This Win11Shell app doesn’t exist.");
        assert_eq!(error("settings", Some(&json!([])), Lang::Nl), "App-parameters moeten een JSON-object zijn.");
        assert_eq!(error("settings", Some(&json!([])), Lang::En), "App parameters must be a JSON object.");
    }
}
