//! Lightweight GIO-only launcher: no GTK initialization or backend.
// The host's i18n runtime (round2-gate.md §2): this bin crate has no `crate::i18n` of the library.
#[allow(dead_code)]
#[path = "../i18n.rs"]
mod i18n;
#[path = "../launch_context.rs"]
mod launch_context;
// GW7: `win11-launch --post-upgrade-check` (design §6.6), GIO only; its terminal goes through process_scope (app.slice)
#[path = "../post_upgrade.rs"]
mod post_upgrade;
#[allow(dead_code)]
#[path = "../process_scope.rs"]
mod process_scope;
use gio::prelude::*;
use serde_json::{Value, json};
use std::{io::Write, os::unix::net::UnixStream};
fn main() {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && args[0] == post_upgrade::FLAG {
        std::process::exit(post_upgrade::run());
    }
    if let Err(error) = run() {
        eprintln!("Win11 app starten: {error}");
        std::process::exit(1)
    }
}
fn run() -> Result<(), String> {
    let mut args = std::env::args();
    let _ = args.next();
    let request: Value = serde_json::from_str(&args.next().ok_or("Launch-opdracht ontbreekt")?)
        .map_err(|e| e.to_string())?;
    if args.next().is_some() {
        return Err("Te veel argumenten".into());
    }
    let result = launch(&request, i18n::current_language());
    if let Some(path) = request["ack"].as_str() {
        if let Ok(mut stream) = UnixStream::connect(path) {
            let response = match &result {
                Ok(()) => json!({"ok":true}),
                Err(error) => json!({"ok":false,"error":error}),
            };
            let _ = stream.write_all(response.to_string().as_bytes());
        }
    }
    result
}
fn launch(request: &Value, lang: i18n::Lang) -> Result<(), String> {
    let token = std::env::var("XDG_ACTIVATION_TOKEN")
        .ok()
        .or_else(|| std::env::var("DESKTOP_STARTUP_ID").ok());
    let context = launch_context::new(token);
    let uris: Vec<&str> = request["uris"]
        .as_array()
        .map(|values| values.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    match request["action"].as_str() {
        Some("desktop") => {
            let info = if let Some(path) = request["filename"].as_str() {
                gio::DesktopAppInfo::from_filename(path)
            } else {
                request["id"].as_str().and_then(gio::DesktopAppInfo::new)
            }
            .ok_or_else(|| crate::i18n::tr(lang, "launch.appNotFound"))?;
            info.launch_uris(&uris, Some(&context))
                .map_err(|e| e.to_string())?;
        }
        Some("default") => {
            let uri = request["uri"].as_str().ok_or_else(|| crate::i18n::tr(lang, "launch.missingUri"))?;
            gio::AppInfo::launch_default_for_uri(uri, Some(&context)).map_err(|e| e.to_string())?;
        }
        _ => return Err(crate::i18n::tr(lang, "launch.unknownRequest").into()),
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    /// launch stem (round2-gate §6): the helper's answers, which go back to the host that launched the app
    /// (files/actions.rs shows them); nl is the 3b898e6 text.
    #[test]
    fn launch_answers_follow_the_settings_language() {
        for (lang, texts) in [
            (
                i18n::Lang::Nl,
                ["Onbekende launch-opdracht", "Bestandsadres ontbreekt", "De app is niet gevonden"],
            ),
            (
                i18n::Lang::En,
                ["Unknown launch request", "The file address is missing", "The app wasn’t found"],
            ),
        ] {
            assert_eq!(launch(&json!({}), lang).unwrap_err(), texts[0]);
            assert_eq!(launch(&json!({"action":"default"}), lang).unwrap_err(), texts[1]);
            let missing = json!({"action":"desktop","id":"win11-r2d-no-such-app.desktop"});
            assert_eq!(launch(&missing, lang).unwrap_err(), texts[2]);
        }
    }
}
