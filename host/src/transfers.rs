//! Acknowledged tab handoff between independent Explorer processes.
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub struct Transfers {
    root: PathBuf,
    pub window_id: String,
}

impl Transfers {
    pub fn new() -> Result<Self, String> {
        let root = gtk::glib::user_runtime_dir().join("win11-gnome/tab-transfers");
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&root)
            .map_err(|e| e.to_string())?;
        let meta = fs::symlink_metadata(&root).map_err(|e| e.to_string())?;
        if !meta.is_dir()
            || meta.file_type().is_symlink()
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.mode() & 0o077 != 0
        {
            return Err("Tab-overdrachtsmap moet privé zijn".into());
        }
        let mut random = [0_u8; 16];
        fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut random))
            .map_err(|e| e.to_string())?;
        let window_id = random.iter().map(|b| format!("{b:02x}")).collect();
        Ok(Self { root, window_id })
    }

    pub fn call(&self, method: &str, args: &Value) -> Result<Value, String> {
        self.call_in(method, args, crate::i18n::current_language())
    }

    /// `call` with its messages in `lang` (the process's settings language in production).
    fn call_in(&self, method: &str, args: &Value, lang: crate::i18n::Lang) -> Result<Value, String> {
        let ticket = args["ticket"]
            .as_str()
            .filter(|s| {
                (32..=80).contains(&s.len())
                    && s.bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
            })
            .ok_or_else(|| crate::i18n::tr(lang, "transfers.invalidTicket"))?;
        let offer_path = self.path(ticket, "offer");
        let claim_path = self.path(ticket, "claim");
        let commit_path = self.path(ticket, "commit");
        if method == "explorer.tabOffer" {
            validate_snapshot(&args["state"], lang)?;
            let offer =
                json!({"state":args["state"],"sourceWindow":self.window_id,"expires":now()+90});
            create(&offer_path, &offer)?;
            return Ok(json!({"ticket":ticket,"sourceWindow":self.window_id}));
        }
        let offer = read_private(&offer_path, lang)?;
        if offer["expires"].as_u64().unwrap_or(0) < now() {
            return Err(crate::i18n::tr(lang, "transfers.expired").into());
        }
        match method {
            "explorer.tabClaim" => {
                if offer["sourceWindow"] == self.window_id {
                    return Err(crate::i18n::tr(lang, "transfers.alreadyHere").into());
                }
                match create(&claim_path, &json!({"window":self.window_id})) {
                    Ok(()) => {}
                    Err(_)
                        if read_private(&claim_path, lang)
                            .is_ok_and(|value| value["window"] == self.window_id) => {}
                    Err(_) => {
                        return Err(crate::i18n::tr(lang, "transfers.claimedElsewhere").into());
                    }
                }
                Ok(json!({"state":offer["state"],"sourceWindow":offer["sourceWindow"]}))
            }
            "explorer.tabCommit" => {
                self.owns_claim(&claim_path, lang)?;
                // A create-only receipt is the only signal that allows closing the source tab.
                if !commit_path.exists() {
                    create(&commit_path, &json!({"window":self.window_id}))?;
                }
                Ok(json!({"committed":true}))
            }
            "explorer.tabRelease" => {
                if offer["sourceWindow"] == self.window_id
                    && !claim_path.exists()
                    && !commit_path.exists()
                {
                    fs::remove_file(offer_path).map_err(|error| error.to_string())?;
                    return Ok(json!({}));
                }
                self.owns_claim(&claim_path, lang)?;
                if commit_path.exists() {
                    return Err(crate::i18n::tr(lang, "transfers.cantWithdraw").into());
                }
                fs::remove_file(claim_path).map_err(|e| e.to_string())?;
                Ok(json!({}))
            }
            "explorer.tabStatus" => {
                if offer["sourceWindow"] != self.window_id {
                    return Err(crate::i18n::tr(lang, "transfers.onlySourceChecks").into());
                }
                Ok(json!({"committed":commit_path.exists()}))
            }
            "explorer.tabDetach" => {
                if offer["sourceWindow"] != self.window_id {
                    return Err(crate::i18n::tr(lang, "transfers.onlySourceDetaches").into());
                }
                if commit_path.exists() {
                    return Ok(json!({}));
                }
                let launching = self.path(ticket, "launch");
                if create(&launching, &json!({"started":now()})).is_ok() {
                    let executable = std::env::current_exe()
                        .map_err(|e| e.to_string())?
                        .with_file_name("win11-explorer");
                    match std::process::Command::new(executable)
                        .args(["--tab-ticket", ticket])
                        .spawn()
                    {
                        Ok(_) => {}
                        Err(e) => {
                            let _ = fs::remove_file(launching);
                            return Err(e.to_string());
                        }
                    }
                }
                Ok(json!({}))
            }
            _ => Err(crate::i18n::tr(lang, "transfers.unknownAction").into()),
        }
    }
    fn owns_claim(&self, path: &Path, lang: crate::i18n::Lang) -> Result<(), String> {
        if read_private(path, lang)?["window"] == self.window_id {
            Ok(())
        } else {
            Err(crate::i18n::tr(lang, "transfers.claimedByOther").into())
        }
    }
    fn path(&self, ticket: &str, kind: &str) -> PathBuf {
        self.root.join(format!("{ticket}.{kind}.json"))
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn create(path: &Path, value: &Value) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}
fn read_private(path: &Path, lang: crate::i18n::Lang) -> Result<Value, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file()
        || meta.file_type().is_symlink()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o077 != 0
        || meta.len() > 2 * 1024 * 1024
    {
        return Err(crate::i18n::tr(lang, "transfers.invalidFile").into());
    }
    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}
fn validate_snapshot(state: &Value, lang: crate::i18n::Lang) -> Result<(), String> {
    if state["version"] != 1
        || !state.is_object()
        || serde_json::to_vec(state).map_err(|e| e.to_string())?.len() > 1024 * 1024
    {
        return Err(crate::i18n::tr(lang, "transfers.invalidContent").into());
    }
    fn location(value: &Value) -> bool {
        match value["kind"].as_str() {
            Some("home" | "gallery" | "thispc" | "trash") => true,
            Some("folder" | "search") => value["path"]
                .as_str()
                .is_some_and(|s| s.starts_with('/') && s.len() <= 4096 && !s.contains('\0')),
            _ => false,
        }
    }
    if !location(&state["loc"])
        || state["history"]
            .as_array()
            .is_some_and(|history| history.len() > 500 || history.iter().any(|loc| !location(loc)))
    {
        return Err(crate::i18n::tr(lang, "transfers.invalidFolder").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_tab_state_without_touching_files() {
        assert!(validate_snapshot(&json!({"version":1,"loc":{"kind":"folder","path":"/tmp/a"},"history":[{"kind":"home"}]}), crate::i18n::Lang::Nl).is_ok());
        assert!(
            validate_snapshot(&json!({"version":1,"loc":{"kind":"folder","path":"relative"}}), crate::i18n::Lang::Nl)
                .is_err()
        );
        assert!(
            validate_snapshot(&json!({"version":1,"loc":{"kind":"script","path":"/tmp/a"}}), crate::i18n::Lang::Nl)
                .is_err()
        );
    }
    #[test]
    fn only_committed_handoffs_may_close_source() {
        let dir = std::env::temp_dir().join(format!("win11-transfer-unit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        let source = Transfers {
            root: dir.clone(),
            window_id: "source".into(),
        };
        let target = Transfers {
            root: dir.clone(),
            window_id: "target".into(),
        };
        let competitor = Transfers {
            root: dir.clone(),
            window_id: "competitor".into(),
        };
        let ticket = "f92de682-9710-4356-8f1c-f47443741776";
        source
            .call(
                "explorer.tabOffer",
                &json!({"ticket":ticket,"state":{"version":1,"loc":{"kind":"home"}}}),
            )
            .unwrap();
        assert_eq!(
            source
                .call("explorer.tabStatus", &json!({"ticket":ticket}))
                .unwrap()["committed"],
            false
        );
        assert!(
            source
                .call("explorer.tabClaim", &json!({"ticket":ticket}))
                .is_err()
        );
        target
            .call("explorer.tabClaim", &json!({"ticket":ticket}))
            .unwrap();
        assert!(
            competitor
                .call("explorer.tabClaim", &json!({"ticket":ticket}))
                .is_err()
        );
        assert!(
            competitor
                .call("explorer.tabCommit", &json!({"ticket":ticket}))
                .is_err()
        );
        target
            .call("explorer.tabRelease", &json!({"ticket":ticket}))
            .unwrap();
        assert_eq!(
            source
                .call("explorer.tabStatus", &json!({"ticket":ticket}))
                .unwrap()["committed"],
            false
        );
        competitor
            .call("explorer.tabClaim", &json!({"ticket":ticket}))
            .unwrap();
        competitor
            .call("explorer.tabCommit", &json!({"ticket":ticket}))
            .unwrap();
        assert_eq!(
            source
                .call("explorer.tabStatus", &json!({"ticket":ticket}))
                .unwrap()["committed"],
            true
        );
        assert!(
            competitor
                .call("explorer.tabRelease", &json!({"ticket":ticket}))
                .is_err()
        );
        fs::remove_dir_all(dir).unwrap();
    }
    /// The en render test of the `transfers` stem (round2-gate §6), through call_in (Transfers::call passes the
    /// process's settings language).
    #[test]
    fn transfer_errors_follow_the_settings_language() {
        use crate::i18n::Lang;
        let dir = std::env::temp_dir().join(format!("win11-transfer-i18n-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        let source = Transfers {
            root: dir.clone(),
            window_id: "source".into(),
        };
        let ticket = "0b3c6a0e-5d0f-4a8e-9a43-2f7d1c6e9b10";
        source
            .call_in(
                "explorer.tabOffer",
                &json!({"ticket":ticket,"state":{"version":1,"loc":{"kind":"home"}}}),
                Lang::Nl,
            )
            .unwrap();
        for (lang, texts) in [
            (Lang::Nl, ["Ongeldige tab-overdrachtscode", "Deze tab staat al in dit venster",
                        "Onbekende tab-overdrachtsactie", "Ongeldige tabinhoud"]),
            (Lang::En, ["Invalid tab transfer code", "This tab is already in this window",
                        "Unknown tab transfer action", "Invalid tab contents"]),
        ] {
            assert_eq!(source.call_in("explorer.tabClaim", &json!({"ticket":"short"}), lang).unwrap_err(), texts[0]);
            assert_eq!(source.call_in("explorer.tabClaim", &json!({"ticket":ticket}), lang).unwrap_err(), texts[1]);
            assert_eq!(source.call_in("explorer.tabNope", &json!({"ticket":ticket}), lang).unwrap_err(), texts[2]);
            assert_eq!(validate_snapshot(&json!({"version":2}), lang).unwrap_err(), texts[3]);
        }
        fs::remove_dir_all(dir).unwrap();
    }
}
