//! Native runtime identity, private configuration import and explicit external oracle attachment.

use gtk::glib;
use serde_json::Value;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone)]
pub struct Backend {
    pub port: u16,
    pub token: String,
}

impl Backend {
    fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 64 * 1024 {
            return Err("Backend-instancebestand is te groot".into());
        }
        let instance: Value = serde_json::from_slice(bytes)
            .map_err(|_| "Backend-instancebestand bevat ongeldige JSON")?;
        let port = instance["port"]
            .as_u64()
            .filter(|port| *port > 0 && *port <= u64::from(u16::MAX))
            .ok_or("Ongeldige backendpoort")? as u16;
        let token = instance["token"]
            .as_str()
            .filter(|token| {
                (16..=256).contains(&token.len())
                    && token
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            })
            .ok_or("Ongeldige backendsleutel")?
            .to_owned();
        Ok(Self { port, token })
    }

    pub fn read(path: &Path) -> Result<Self, String> {
        let metadata = fs::symlink_metadata(path)
            .map_err(|err| format!("Backend-instancebestand {}: {err}", path.display()))?;
        let parent = path
            .parent()
            .ok_or("Backend-instancebestand mist een map")?;
        let directory = fs::symlink_metadata(parent)
            .map_err(|err| format!("Backend-instancemap {}: {err}", parent.display()))?;
        // Match the reference host's trust boundary: a local user cannot replace
        // our instance file with their own listener or inject HTTP header bytes.
        let uid = unsafe { libc::geteuid() };
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.uid() != uid
            || metadata.mode() & 0o077 != 0
            || !directory.is_dir()
            || directory.file_type().is_symlink()
            || directory.uid() != uid
            || directory.mode() & 0o077 != 0
        {
            return Err(
                "Backend-instancebestand en map moeten privé en van deze gebruiker zijn".into(),
            );
        }
        let mut bytes = Vec::new();
        File::open(path)
            .and_then(|file| file.take(64 * 1024 + 1).read_to_end(&mut bytes))
            .map_err(|err| format!("Backend-instancebestand lezen: {err}"))?;
        Self::parse(&bytes)
    }

    fn is_ready(&self) -> bool {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_millis(300))
            .timeout_read(Duration::from_millis(600))
            .build();
        agent
            .post(&format!(
                "http://127.0.0.1:{}/api/call/shell.info",
                self.port
            ))
            .set("X-Shell-Token", &self.token)
            .send_json(serde_json::json!({"args": {}}))
            .ok()
            .and_then(|response| response.into_json::<Value>().ok())
            .is_some_and(|response| response["ok"] == true)
    }
}

/// A native runtime has port zero: every API is served by Rust. An explicit external
/// instance is only an oracle/compatibility attachment and is never started or stopped here.
pub struct BackendGuard {
    pub backend: Backend,
}
impl BackendGuard {
    pub fn connect(_vendor_root: &Path, _kind: &str) -> Result<Self, String> {
        if let Some(path) = std::env::var_os("WIN11_REFERENCE_INSTANCE") {
            let backend = Backend::read(Path::new(&path))?;
            if !backend.is_ready() {
                return Err("De expliciet gekozen referentiedienst reageert niet".into());
            }
            return Ok(Self { backend });
        }
        let config = std::env::var_os("WIN11_GNOME_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| glib::user_config_dir().join("win11-gnome"));
        private_directory(&config)?;
        seed_settings(&config)?;
        let mut random = [0u8; 32];
        File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut random))
            .map_err(|e| format!("Veilige toegangssleutel maken: {e}"))?;
        let token = random.iter().map(|b| format!("{b:02x}")).collect();
        Ok(Self {
            backend: Backend { port: 0, token },
        })
    }
}
/// Shared by API dispatch, service helpers and side effects. No process-wide mutable switch.
pub(crate) fn dry_run() -> bool {
    std::env::args().any(|arg| arg == "--dry-run")
        || std::env::var("WIN11SHELL_DRY_RUN").is_ok_and(|v| {
            !v.is_empty()
                && !matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "0" | "false" | "no" | "off"
                )
        })
}
/// Private discovery for --http-only / integration tests; tokens never go to stdout or argv.
pub(crate) fn write_instance(path: &Path, port: u16, token: &str) -> Result<(), String> {
    let parent = path.parent().ok_or("Instancebestand mist een map")?;
    private_directory(parent)?;
    let tmp = parent.join(format!(".instance-{}", unique_suffix()));
    let result = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        let data = serde_json::json!({"port":port,"token":token,"pid":std::process::id(),"runtime":"rust","dryRun":dry_run()});
        f.write_all(data.to_string().as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(|e: std::io::Error| e.to_string())
}
pub(crate) fn remove_instance(path: &Path) {
    if fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .is_some_and(|v| v["pid"] == std::process::id())
    {
        let _ = fs::remove_file(path);
    }
}

fn private_directory(path: &Path) -> Result<(), String> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|err| format!("Privémap {} maken: {err}", path.display()))?;
    let metadata =
        fs::symlink_metadata(path).map_err(|err| format!("Privémap controleren: {err}"))?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
    {
        return Err(format!("{} moet een eigen map zijn", path.display()));
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|err| format!("Privémap beveiligen: {err}"))
}

fn seed_settings(config: &Path) -> Result<(), String> {
    let destination = config.join("settings.json");
    if destination.exists() {
        return Ok(());
    }
    let reference = std::env::var_os("WIN11_REFERENCE_SETTINGS")
        .map(PathBuf::from)
        .unwrap_or_else(|| glib::home_dir().join(".config/win11shell/settings.json"));
    if !reference.is_file() {
        return Ok(());
    } // Reference service supplies its defaults.
    let bytes =
        fs::read(&reference).map_err(|err| format!("Bestaande instellingen lezen: {err}"))?;
    serde_json::from_slice::<Value>(&bytes)
        .map_err(|_| "Bestaande instellingen bevatten ongeldige JSON")?;
    let temporary = config.join(format!(".settings-import-{}", unique_suffix()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        // Atomic create-only publication: another app's configuration is never replaced.
        match fs::hard_link(&temporary, &destination) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            Err(err) => Err(err),
        }
    })();
    let _ = fs::remove_file(&temporary);
    result.map_err(|err: std::io::Error| format!("Eigen instellingen initialiseren: {err}"))
}

fn unique_suffix() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_rejects_invalid_ports_and_header_injection() {
        for port in [0, 65536] {
            let bytes = serde_json::json!({"port":port,"token":"abcdefghijklmnop"}).to_string();
            assert!(Backend::parse(bytes.as_bytes()).is_err());
        }
        let bytes =
            serde_json::json!({"port":1234,"token":"abcdefghijklmnop\r\nX-Evil: 1"}).to_string();
        assert!(Backend::parse(bytes.as_bytes()).is_err());
        let valid = Backend::parse(br#"{"port":37457,"token":"abc_DEF-123456789"}"#).unwrap();
        assert_eq!(valid.port, 37457);
    }

    #[test]
    fn initial_settings_import_never_replaces_existing_settings() {
        let root = std::env::temp_dir().join(format!("win11-config-test-{}", unique_suffix()));
        private_directory(&root).unwrap();
        let original = b"{\"theme\":{\"mode\":\"light\"}}";
        fs::write(root.join("settings.json"), original).unwrap();
        seed_settings(&root).unwrap();
        assert_eq!(fs::read(root.join("settings.json")).unwrap(), original);
        fs::remove_dir_all(root).unwrap();
    }
}
