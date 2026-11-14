//! The file portal's picker (portal.rs): the native GTK 4 Explorer in picker mode, and the check of its answer.
use serde_json::{Value, json};
use std::path::Path;
/// The native GTK 4 picker of a portal request: `win11-gtk-explorer --picker-stdio` from this binary's directory
/// (native/apps/explorer/src/picker.rs), on the portal's own backend (WIN11_APP_BACKEND), parented to the
/// caller's exported window itself. One JSON line goes to its stdin (`{picker, path, parent}`); its answers come
/// on stdout and get the [`selection`] check: a valid one is acknowledged (`{"ok":true}`, the picker closes) and
/// sent to the portal, an invalid one goes back as `{"error"}` (shown in the picker, which stays open). A close or
/// the end of its stdout is a cancel. Dropping the handle ends the process (an answer, Request.Close, the caller
/// gone, the portal stopping). None: the native binary is missing (said on stderr, as for an app) or did not
/// start; the portal then answers the request as failed.
pub(crate) struct NativePicker {
    child: Option<std::process::Child>,
    accepted: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for NativePicker {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        // a picker that got its acknowledgement closes by itself (and saves its size); others are told to go
        let accepted = self.accepted.load(std::sync::atomic::Ordering::Acquire);
        let _ = std::thread::Builder::new()
            .name("win11-picker-reaper".into())
            .spawn(move || {
                if !accepted && let Ok(pid) = i32::try_from(child.id()) {
                    unsafe {
                        libc::kill(pid, libc::SIGTERM);
                    }
                }
                for _ in 0..40 {
                    if !matches!(child.try_wait(), Ok(None)) {
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                let _ = child.kill();
                let _ = child.wait();
            });
    }
}

/// `backend` is the portal's own, as `<port>:<token>`.
pub(crate) fn show_native(
    options: &Value,
    parent: &str,
    backend: &str,
) -> Option<(
    NativePicker,
    std::sync::mpsc::Receiver<Result<Value, String>>,
)> {
    use std::io::{BufRead, Read, Write};
    use std::os::unix::process::CommandExt;
    let binary =
        crate::exec_path(std::env::current_exe().ok()?).with_file_name("win11-gtk-explorer");
    if !binary.is_file() {
        eprintln!(
            "{}",
            crate::i18n::trf(
                crate::i18n::current_language(),
                "core.nativeAppMissing",
                &json!({"path": binary.display().to_string()})
            )
        );
        return None;
    }
    let directory = options["directory"]
        .as_str()
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute() && p.is_dir())
        .unwrap_or_else(glib::home_dir);
    let request = json!({"picker":options,"path":directory.to_string_lossy(),"parent":parent});
    let mut command = std::process::Command::new(&binary);
    command
        .arg("--picker-stdio")
        .env("WIN11_APP_BACKEND", backend)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit());
    // Like this process (GTK_USE_PORTAL=0, run()), the picker's GTK 4 must not ask the portal frontend that is
    // waiting for this very request (its settings portal).
    if std::env::var_os("GDK_DEBUG").is_none() {
        command.env("GDK_DEBUG", "no-portals");
    }
    // SAFETY: pre_exec runs in the forked child before exec; prctl is async-signal-safe. The picker ends with
    // the portal (the spawning thread is the portal's main thread).
    unsafe {
        command.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            eprintln!("Win11 file portal: {}: {error}", binary.display());
            return None;
        }
    };
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    };
    let accepted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = accepted.clone();
    let options = options.clone();
    let (ack, receiver) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("win11-picker-native".into())
        .spawn(move || {
            let mut answer = None;
            if writeln!(stdin, "{request}").and_then(|_| stdin.flush()).is_ok() {
                let mut reader = std::io::BufReader::new(stdout);
                loop {
                    let mut line = String::new();
                    if !matches!(reader.by_ref().take(16 << 20).read_line(&mut line), Ok(n) if n > 0) {
                        break;
                    }
                    let Ok(message) = serde_json::from_str::<Value>(line.trim()) else {
                        continue;
                    };
                    match message["cmd"].as_str() {
                        Some("picker-submit") => {
                            match selection(&message, &options, crate::i18n::current_language()) {
                                Ok(value) => {
                                    flag.store(true, std::sync::atomic::Ordering::Release);
                                    let _ = writeln!(stdin, "{}", json!({"ok":true}));
                                    let _ = stdin.flush();
                                    answer = Some(value);
                                    break;
                                }
                                Err(error) => {
                                    let _ = writeln!(stdin, "{}", json!({"error":error}));
                                    let _ = stdin.flush();
                                }
                            }
                        }
                        Some("close") => break,
                        _ => {}
                    }
                }
            }
            let _ = ack.send(Ok(
                answer.unwrap_or_else(|| json!({"path":null,"paths":[],"cancelled":true}))
            ));
        });
    let picker = NativePicker {
        child: Some(child),
        accepted,
    };
    if spawned.is_err() {
        return None;
    }
    Some((picker, receiver))
}

fn selection(value: &Value, options: &Value, lang: crate::i18n::Lang) -> Result<Value, String> {
    let folder = options["folder"] == true;
    let save = !folder && (options["save"] == true || options["action"] == "save");
    let multiple = options["multiple"] == true && !save;
    let selected = if let Some(paths) = value["paths"].as_array() {
        paths.clone()
    } else {
        vec![value.get("path").unwrap_or(value).clone()]
    };
    if selected.is_empty() || selected.len() > 1000 || (!multiple && selected.len() != 1) {
        return Err(crate::i18n::tr(lang, "picker.chooseValidSelection").into());
    }
    let mut paths = Vec::new();
    for path in selected {
        let path = path
            .as_str()
            .filter(|p| !p.is_empty() && p.len() <= 8192 && !p.contains('\0'))
            .ok_or_else(|| crate::i18n::tr(lang, "picker.chooseValidPath"))?;
        validate(Path::new(path), folder, save, lang)?;
        if !paths.iter().any(|v| v == path) {
            paths.push(path.to_owned());
        }
    }
    let filters = options["filters"].as_array().map(Vec::len).unwrap_or(0);
    let index = value["filterIndex"]
        .as_u64()
        .unwrap_or_else(|| options["currentFilter"].as_u64().unwrap_or(0));
    if index >= filters.max(1) as u64 {
        return Err(crate::i18n::tr(lang, "picker.invalidFileType").into());
    }
    let supplied = value["choices"].as_array().cloned().unwrap_or_default();
    let mut choices = Vec::new();
    for choice in options["choices"].as_array().into_iter().flatten() {
        let Some(id) = choice["id"].as_str() else {
            continue;
        };
        let selected = supplied
            .iter()
            .find(|v| v[0].as_str() == Some(id))
            .map(|v| &v[1])
            .unwrap_or(&choice["selected"]);
        let selected = selected
            .as_str()
            .ok_or_else(|| crate::i18n::tr(lang, "picker.invalidOption"))?;
        let items = choice["options"].as_array().cloned().unwrap_or_default();
        if if items.is_empty() {
            !matches!(selected, "true" | "false")
        } else {
            !items.iter().any(|v| v["id"].as_str() == Some(selected))
        } {
            return Err(crate::i18n::tr(lang, "picker.invalidOption").into());
        }
        choices.push(json!([id, selected]));
    }
    let writable = if options["allowReadOnly"] == true {
        value["writable"].as_bool().unwrap_or(true)
    } else {
        options["writable"] != false
    };
    Ok(
        json!({"path":paths[0],"paths":paths,"filterIndex":index,"choices":choices,"writable":writable}),
    )
}
fn validate(path: &Path, folder: bool, save: bool, lang: crate::i18n::Lang) -> Result<(), String> {
    if !path.is_absolute() {
        return Err(crate::i18n::tr(lang, "picker.chooseAbsolutePath").into());
    }
    if folder {
        if !path.is_dir() {
            return Err(crate::i18n::tr(lang, "picker.chooseExistingFolder").into());
        }
    } else if save {
        if path.is_dir() || !path.parent().is_some_and(Path::is_dir) {
            return Err(crate::i18n::tr(lang, "picker.chooseFileInFolder").into());
        }
    } else if !path.is_file() {
        return Err(crate::i18n::tr(lang, "picker.chooseExistingFile").into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Lang;
    fn root() -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("picker-selection-{}", crate::events::nonce()));
        std::fs::create_dir(&p).unwrap();
        p
    }
    #[test]
    fn multiple_selections_are_validated_and_deduplicated() {
        let p = root();
        let a = p.join("a");
        let b = p.join("b");
        std::fs::write(&a, "").unwrap();
        std::fs::write(&b, "").unwrap();
        assert!(selection(&json!({"paths":[a,b]}), &json!({}), Lang::Nl).is_err());
        let v = selection(
            &json!({"paths":[a,a,b]}),
            &json!({"multiple":true}),
            Lang::Nl,
        )
        .unwrap();
        assert_eq!(v["paths"], json!([a, b]));
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn saved_selection_never_creates_a_file_and_validates_options() {
        let p = root();
        let a = p.join("new document.txt");
        let o = json!({"save":true,"filters":[{"patterns":["*.txt"]}],"choices":[{"id":"encoding","options":[{"id":"utf8","label":"UTF-8"}],"selected":"utf8"}],"allowReadOnly":true});
        let v = selection(
            &json!({"path":a,"filterIndex":0,"choices":[["encoding","utf8"]],"writable":false}),
            &o,
            Lang::Nl,
        )
        .unwrap();
        assert_eq!(v["writable"], false);
        assert!(!a.exists());
        assert!(selection(&json!({"path":a,"filterIndex":1}), &o, Lang::Nl).is_err());
        assert!(
            selection(
                &json!({"path":a,"choices":[["encoding","bad"]]}),
                &o,
                Lang::Nl
            )
            .is_err()
        );
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn folders_and_invalid_paths_remain_separate() {
        let p = root();
        let a = p.join("a");
        std::fs::write(&a, "").unwrap();
        assert!(selection(&json!({"path":p}), &json!({"folder":true}), Lang::Nl).is_ok());
        assert!(selection(&json!({"path":a}), &json!({"folder":true}), Lang::Nl).is_err());
        assert!(selection(&json!({"path":"relative"}), &json!({"save":true}), Lang::Nl).is_err());
        assert!(
            selection(
                &json!({"path":"/tmp/a\0b"}),
                &json!({"save":true}),
                Lang::Nl
            )
            .is_err()
        );
        std::fs::remove_dir_all(p).unwrap();
    }
    /// The en render test of the `picker` stem (round2-gate §6): the selection checks of a submitted picker.
    #[test]
    fn selection_errors_follow_the_settings_language() {
        let p = root();
        let file = p.join("a");
        std::fs::write(&file, "").unwrap();
        let error = |value: Value, options: Value, lang: Lang| {
            selection(&value, &options, lang).unwrap_err()
        };
        for (lang, texts) in [
            (
                Lang::Nl,
                [
                    "Kies een geldige bestandsselectie.",
                    "Kies een absoluut bestandspad.",
                    "Kies een bestaande map.",
                    "Kies een bestaand bestand.",
                    "Ongeldig bestandstype.",
                ],
            ),
            (
                Lang::En,
                [
                    "Choose a valid file selection.",
                    "Choose an absolute file path.",
                    "Choose an existing folder.",
                    "Choose an existing file.",
                    "Invalid file type.",
                ],
            ),
        ] {
            assert_eq!(error(json!({"paths":[]}), json!({}), lang), texts[0]);
            assert_eq!(error(json!({"path":"relative"}), json!({}), lang), texts[1]);
            assert_eq!(
                error(json!({"path":file}), json!({"folder":true}), lang),
                texts[2]
            );
            assert_eq!(error(json!({"path":p}), json!({}), lang), texts[3]);
            assert_eq!(
                error(json!({"path":file,"filterIndex":3}), json!({}), lang),
                texts[4]
            );
        }
        std::fs::remove_dir_all(p).unwrap();
    }
}
