//! Only the installer-owned executable and generated launcher are ever removed.
use super::{Paths, common::*};
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
fn manifest(paths: &Paths) -> std::io::Result<Value> {
    match read_json(&paths.state.join("appimages.json"), 2 << 20) {
        Ok(v) if v["version"] == 1 && v["apps"].is_object() => Ok(v),
        Ok(_) => Err(std::io::Error::other("Invalid AppImage manifest")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({"version":1,"apps":{}})),
        Err(e) => Err(e),
    }
}
fn expected_id(desktop: &str) -> Option<&str> {
    desktop
        .strip_prefix("win11shell-appimage-")?
        .strip_suffix(".desktop")
        .filter(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}
fn image_path(paths: &Paths, id: &str) -> PathBuf {
    paths.managed.join(id).join("application.AppImage")
}
fn desktop_path(paths: &Paths, id: &str) -> PathBuf {
    paths
        .applications
        .join(format!("win11shell-appimage-{id}.desktop"))
}
fn exec(path: &Path) -> String {
    let s = path.to_string_lossy();
    let s = s
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$")
        .replace('%', "%%");
    format!("\"{s}\" %U")
}
pub(super) fn record(paths: &Paths, desktop: &str) -> Option<Value> {
    let id = expected_id(desktop)?;
    let value = manifest(paths).ok()?["apps"][id].clone();
    if value["id"] == id && value["desktopId"] == desktop {
        Some(value)
    } else {
        None
    }
}
pub(super) fn validate(paths: &Paths, value: &Value) -> Result<(), String> {
    let id = value["id"]
        .as_str()
        .filter(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or("Invalid managed AppImage id")?;
    let dir = paths.managed.join(id);
    let metadata = fs::symlink_metadata(&dir).map_err(|e| e.to_string())?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o022 != 0
    {
        return Err("The managed application directory changed".into());
    }
    let path = image_path(paths, id);
    if let Ok(m) = fs::symlink_metadata(&path) {
        if !m.is_file()
            || m.file_type().is_symlink()
            || m.uid() != unsafe { libc::geteuid() }
            || m.nlink() != 1
        {
            return Err("The managed application file changed".into());
        }
    }
    let path = desktop_path(paths, id);
    if let Ok(m) = fs::symlink_metadata(&path) {
        if !m.is_file()
            || m.file_type().is_symlink()
            || m.uid() != unsafe { libc::geteuid() }
            || m.nlink() != 1
        {
            return Err("The managed application launcher changed".into());
        }
        let key = glib::KeyFile::new();
        key.load_from_file(&path, glib::KeyFileFlags::NONE)
            .map_err(|e| e.to_string())?;
        if key
            .string("Desktop Entry", "X-Win11Shell-AppImageId")
            .ok()
            .as_deref()
            != Some(id)
            || key.string("Desktop Entry", "Exec").ok().as_deref()
                != Some(exec(&image_path(paths, id)).as_str())
        {
            return Err("The managed application launcher changed".into());
        }
    }
    Ok(())
}
pub(super) fn install(
    paths: &Paths,
    stage: &Path,
    info: &Value,
    cancel: &AtomicBool,
    update: &dyn Fn(u32),
) -> Result<Value, String> {
    private_dir(&paths.managed).map_err(|e| e.to_string())?;
    user_dir(&paths.applications).map_err(|e| e.to_string())?;
    let ident = id().map_err(|e| e.to_string())?;
    let dir = paths.managed.join(&ident);
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&dir)
        .map_err(|e| e.to_string())?;
    let created_desktop = std::cell::Cell::new(false);
    let created_target = std::cell::Cell::new(false);
    let target = image_path(paths, &ident);
    let desktop = desktop_path(paths, &ident);
    let result = (|| {
        let mut source = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(stage)
            .map_err(|e| e.to_string())?;
        let size = source.metadata().map_err(|e| e.to_string())?.len();
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&target)
            .map_err(|e| e.to_string())?;
        created_target.set(true);
        let mut checksum =
            glib::Checksum::new(glib::ChecksumType::Sha256).ok_or("SHA256 unavailable")?;
        let mut buffer = [0u8; 128 * 1024];
        let (mut done, mut last) = (0u64, 0u32);
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err("cancelled_copy".into());
            }
            let n = source.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            out.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
            checksum.update(&buffer[..n]);
            done += n as u64;
            let pc = ((done.saturating_mul(90)) / size.max(1)) as u32;
            if pc != last {
                update(pc.min(90));
                last = pc;
            }
        }
        if checksum.string().as_deref() != info["_sha256"].as_str() {
            return Err("The installation file changed".into());
        }
        out.set_permissions(fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
        out.sync_all().map_err(|e| e.to_string())?;
        drop(out);
        if cancel.load(Ordering::Acquire) {
            return Err("cancelled_copy".into());
        }
        let title = clean(info["name"].as_str().unwrap_or("AppImage"), 200);
        let key = glib::KeyFile::new();
        for (k, v) in [
            ("Type", "Application"),
            ("Name", title.as_str()),
            ("Exec", exec(&target).as_str()),
            ("TryExec", target.to_string_lossy().as_ref()),
            ("Icon", "application-x-executable"),
            ("Terminal", "false"),
            ("Categories", "Utility;"),
            ("X-Win11Shell-AppImageId", ident.as_str()),
        ] {
            key.set_string("Desktop Entry", k, v);
        }
        let _lock = lock(&paths.state).map_err(|e| e.to_string())?;
        if desktop.exists() {
            return Err("Application launcher already exists".into());
        }
        atomic_new(&desktop, key.to_data().as_bytes(), 0o644).map_err(|e| e.to_string())?;
        created_desktop.set(true);
        let record = json!({"id":ident,"desktopId":format!("win11shell-appimage-{ident}.desktop"),"name":title,"version":info["version"],"sha256":info["_sha256"],"installedAt":now()});
        let mut all = manifest(paths).map_err(|e| e.to_string())?;
        all["apps"][&ident] = record.clone();
        write_json(&paths.state.join("appimages.json"), &all).map_err(|e| e.to_string())?;
        Ok(record)
    })();
    if result.is_err() {
        if created_desktop.get() {
            let _ = fs::remove_file(&desktop);
        }
        if created_target.get() {
            let _ = fs::remove_file(&target);
        }
        let _ = fs::remove_dir(&dir);
    }
    result
}
pub(super) fn remove(paths: &Paths, value: &Value) -> Result<(), String> {
    let _lock = lock(&paths.state).map_err(|e| e.to_string())?;
    let id = value["id"].as_str().ok_or("Invalid application")?;
    let mut all = manifest(paths).map_err(|e| e.to_string())?;
    if all["apps"][id].is_null() {
        return Err("The managed application is no longer registered".into());
    }
    validate(paths, &all["apps"][id])?;
    for path in [desktop_path(paths, id), image_path(paths, id)] {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    } // Unexpected files, including user documents, are deliberately left in the directory.
    let _ = fs::remove_dir(paths.managed.join(id));
    all["apps"].as_object_mut().unwrap().remove(id);
    write_json(&paths.state.join("appimages.json"), &all).map_err(|e| e.to_string())
}
