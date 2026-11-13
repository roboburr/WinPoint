//! Launch-scoped GTK portal opt-in after validating this user's owned registration.
use serde_json::Value;
use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};
const NAME: &str = "org.freedesktop.impl.portal.desktop.win11shell";
const IFACE: &str = "org.freedesktop.impl.portal.FileChooser";
const UNIT: &str = "app-win11-file-portal.service";

fn read(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } || meta.len() > limit {
        return None;
    }
    let mut data = Vec::new();
    file.take(limit + 1).read_to_end(&mut data).ok()?;
    (data.len() as u64 <= limit).then_some(data)
}
fn owned(entry: &Value, expected: &Path) -> Option<Vec<u8>> {
    if Path::new(entry["path"].as_str()?) != expected {
        return None;
    }
    let encoded = entry["owned"].as_str()?;
    if encoded.len() > 128 * 1024 {
        return None;
    }
    let wanted = glib::base64_decode(encoded);
    if glib::base64_encode(&wanted).as_str() != encoded {
        return None;
    }
    let actual = read(expected, 96 * 1024)?;
    (actual == wanted).then_some(actual)
}
fn value(raw: &[u8], group: &str, key: &str) -> Option<String> {
    let file = glib::KeyFile::new();
    file.load_from_data(std::str::from_utf8(raw).ok()?, glib::KeyFileFlags::NONE)
        .ok()?;
    Some(file.string(group, key).ok()?.to_string())
}
fn raw_value(raw: &[u8], group: &str, key: &str) -> Option<String> {
    let file = glib::KeyFile::new();
    file.load_from_data(std::str::from_utf8(raw).ok()?, glib::KeyFileFlags::NONE)
        .ok()?;
    Some(file.value(group, key).ok()?.to_string())
}
fn executable(path: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o111 == 0 {
        return false;
    }
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let mut magic = [0; 4];
    file.read_exact(&mut magic).is_ok() && magic == *b"\x7fELF"
}
fn registration_at(prefix: &Path, data: &Path, config: &Path) -> bool {
    let check = (|| -> Option<()> {
        let payload = prefix.join("share/win11-gnome");
        if read(&payload.join(".win11-gnome-owned"), 128)?.as_slice()
            != b"win11-gnome-rust-host-v1\n"
        {
            return None;
        }
        let manifest: Value =
            serde_json::from_slice(&read(&payload.join("file-portal.json"), 512 * 1024)?).ok()?;
        if manifest["marker"] != "win11-file-portal-v1" {
            return None;
        }
        let files = &manifest["files"];
        let definition = owned(
            &files["definition"],
            &data.join("xdg-desktop-portal/portals/win11shell.portal"),
        )?;
        if value(&definition, "portal", "DBusName")?.as_str() != NAME
            || value(&definition, "portal", "Interfaces")?
                .split(';')
                .filter(|v| !v.is_empty())
                .collect::<Vec<_>>()
                != [IFACE]
        {
            return None;
        }
        let activation = owned(
            &files["activation"],
            &data.join("dbus-1/services").join(format!("{NAME}.service")),
        )?;
        if value(&activation, "D-BUS Service", "Name")?.as_str() != NAME
            || value(&activation, "D-BUS Service", "SystemdService")?.as_str() != UNIT
        {
            return None;
        }
        let args = glib::shell_parse_argv(value(&activation, "D-BUS Service", "Exec")?).ok()?;
        let binary = prefix.join("bin/win11-file-portal");
        if args.len() != 1 || Path::new(args[0].as_os_str()) != binary || !executable(&binary) {
            return None;
        }
        let unit = owned(&files["unit"], &config.join("systemd/user").join(UNIT))?;
        let escaped = binary
            .to_str()?
            .replace('\\', r"\\")
            .replace('"', r#"\""#)
            .replace('$', "$$")
            .replace('%', "%%");
        let expected_exec = format!(r#"/usr/bin/env -- "{escaped}""#);
        if value(&unit, "Service", "Type")?.as_str() != "dbus"
            || value(&unit, "Service", "BusName")?.as_str() != NAME
            || raw_value(&unit, "Service", "ExecStart")? != expected_exec
        {
            return None;
        }
        let selector = PathBuf::from(files["selector"]["path"].as_str()?);
        let filename = selector.file_name()?.to_str()?;
        if selector.parent() != Some(config.join("xdg-desktop-portal").as_path())
            || !filename.ends_with("portals.conf")
            || !filename
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        {
            return None;
        }
        let selected = value(&read(&selector, 128 * 1024)?, "preferred", IFACE)?;
        if selected.split(';').next()?.trim() != "win11shell" {
            return None;
        }
        Some(())
    })();
    check.is_some()
}
fn registration_present() -> bool {
    let prefix = std::env::var_os("WIN11_INSTALL_PREFIX")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::current_exe()
                .ok()?
                .parent()?
                .parent()
                .map(Path::to_path_buf)
        });
    prefix.is_some_and(|prefix| {
        registration_at(&prefix, &glib::user_data_dir(), &glib::user_config_dir())
    })
}
pub(super) fn captured() -> Option<Option<OsString>> {
    (std::env::var("WIN11_PORTAL_ENV").as_deref() == Ok("1"))
        .then(|| std::env::var_os("WIN11_PORTAL_PARENT_GTK_USE_PORTAL"))
}
pub(super) fn default_opt_in() -> bool {
    std::env::var_os("GTK_USE_PORTAL").is_none() && registration_present()
}
#[cfg(test)]
#[path = "portal/tests.rs"]
mod tests;
