use super::{Paths, Tools, appimage, common::*, snapd};
use crate::files::{ApiError, ApiResult};
use gio::prelude::*;
use serde_json::{Value, json};
use std::{
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
fn blocked(mut v: Value, lang: &str, code: &str, nl: &str, en: &str) -> Value {
    v["supported"] = json!(false);
    v["reasonCode"] = json!(code);
    v["reason"] = json!(t(lang, nl, en));
    v
}
fn base(id: &str, name: &str) -> Value {
    json!({"id":id,"name":name,"provider":"unknown","scope":"system","supported":false,"action":"uninstall","description":""})
}
pub(super) fn resolve(
    id: &str,
    known: &Value,
    paths: &Paths,
    tools: &Tools,
    lang: &str,
) -> ApiResult {
    if [
        "settings",
        "explorer",
        "taskmgr",
        "alles",
        "disks",
        "defaultapps",
        "installer",
    ]
    .contains(&id)
        && known
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v["id"] == id))
    {
        return Ok(blocked(
            base(
                id,
                known
                    .as_array()
                    .and_then(|a| a.iter().find(|v| v["id"] == id))
                    .and_then(|v| v["name"].as_str())
                    .unwrap_or(id),
            ),
            lang,
            "protected",
            "Win11Shell zelf kan hier niet worden verwijderd.",
            "Win11Shell itself cannot be removed here.",
        ));
    }
    if id.is_empty()
        || id.len() > 300
        || !id.ends_with(".desktop")
        || id.contains('/')
        || id.contains('\0')
    {
        return Err(err(
            lang,
            "not_found",
            "Deze app is niet meer geïnstalleerd.",
            "This application is no longer installed.",
        ));
    }
    let app = gio::DesktopAppInfo::new(id).ok_or_else(|| {
        err(
            lang,
            "not_found",
            "De app-snelkoppeling bestaat niet meer.",
            "The application launcher no longer exists.",
        )
    })?;
    let mut v = base(id, &app.name());
    if let Some(record) = appimage::record(paths, id) {
        v["provider"] = json!("appimage");
        v["scope"] = json!("user");
        v["description"] = json!(t(
            lang,
            "Verwijdert de beheerde AppImage-kopie en snelkoppeling. Het oorspronkelijke downloadbestand blijft staan.",
            "Removes the managed AppImage copy and launcher. The original download is kept."
        ));
        v["_record"] = record.clone();
        return Ok(match appimage::validate(paths, &record) {
            Ok(()) => {
                v["supported"] = json!(true);
                v
            }
            Err(_) => blocked(
                v,
                lang,
                "changed",
                "De beheerde app is gewijzigd en kan niet veilig worden verwijderd.",
                "The managed application changed and cannot be safely removed.",
            ),
        });
    }
    if id.starts_with("io.github.win11shell.")
        || [
            "win11-settings.desktop",
            "win11-explorer.desktop",
            "win11-task-manager.desktop",
            "win11-search.desktop",
            "win11-disks.desktop",
            "win11-default-apps.desktop",
            "win11-installer.desktop",
        ]
        .contains(&id)
    {
        return Ok(blocked(
            v,
            lang,
            "protected",
            "Win11Shell zelf kan hier niet worden verwijderd.",
            "Win11Shell itself cannot be removed here.",
        ));
    }
    let raw = app.filename().ok_or_else(|| {
        err(
            lang,
            "not_found",
            "De app-snelkoppeling ontbreekt.",
            "The application launcher is missing.",
        )
    })?;
    // A user override is only a shortcut. Package ownership of its symlink target must never turn this into system package removal.
    if raw.starts_with(&paths.applications) {
        let m = std::fs::symlink_metadata(&raw)
            .map_err(|e| ApiError::new("no_access", &e.to_string()))?;
        v["provider"] = json!("shortcut");
        v["scope"] = json!("user");
        v["action"] = json!("removeShortcut");
        v["supported"] = json!(
            m.uid() == unsafe { libc::geteuid() } && (m.is_file() || m.file_type().is_symlink())
        );
        v["description"] = json!(t(
            lang,
            "Verwijdert alleen deze snelkoppeling. App-bestanden en persoonlijke gegevens blijven staan.",
            "Removes this launcher only. Application files and personal data are kept."
        ));
        v["_launcher"] = json!(raw);
        v["_identity"] = json!({"dev":m.dev(),"ino":m.ino(),"mtime":m.mtime(),"mtimeNsec":m.mtime_nsec(),"size":m.len()});
        return Ok(v);
    }
    let path = raw
        .canonicalize()
        .map_err(|e| ApiError::new("no_access", &e.to_string()))?;
    let flat_id = app.string("X-Flatpak").map(|s| s.to_string());
    let user_flat = glib::user_data_dir().join("flatpak");
    let system_flat = Path::new("/var/lib/flatpak");
    if let Some(fid) = flat_id.filter(|s| flatpak_id(s)) {
        let scope = if path.starts_with(&user_flat) {
            "user"
        } else if path.starts_with(system_flat) {
            "system"
        } else {
            ""
        };
        if !scope.is_empty() {
            v["provider"] = json!("flatpak");
            v["scope"] = json!(scope);
            let mut cmd = Command::new(&tools.flatpak);
            let root = if scope == "user" {
                user_flat.as_path()
            } else {
                system_flat
            };
            let exact = path.strip_prefix(root).ok().and_then(|p| {
                let parts = p
                    .iter()
                    .take(4)
                    .map(|v| v.to_string_lossy().into_owned())
                    .collect::<Vec<_>>();
                let r = parts.join("/");
                (valid_ref(&r) && parts.get(1).map(String::as_str) == Some(fid.as_str()))
                    .then_some(r)
            });
            cmd.arg("info")
                .arg(format!("--{scope}"))
                .arg("--show-ref")
                .arg(exact.as_deref().unwrap_or(&fid));
            if let Ok(out) = bounded(cmd, Duration::from_secs(8), 4096) {
                let reference = String::from_utf8_lossy(&out).trim().to_string();
                if valid_ref(&reference) && reference.split('/').nth(1) == Some(fid.as_str()) {
                    v["_ref"] = json!(reference);
                    v["supported"] = json!(true);
                    v["description"] = json!(t(
                        lang,
                        "Verwijdert de Flatpak-app. Persoonlijke app-gegevens blijven staan.",
                        "Removes the Flatpak application. Personal application data is kept."
                    ));
                    return Ok(v);
                }
            }
            return Ok(blocked(
                v,
                lang,
                "provider_unavailable",
                "Flatpak kon deze geïnstalleerde app niet vinden.",
                "Flatpak could not resolve this installed application.",
            ));
        }
    }
    if raw.starts_with("/var/lib/snapd/desktop/applications") {
        if let Some(name) = app.string("X-SnapInstanceName").filter(|s| snap_name(s)) {
            if id.starts_with(&format!("{name}_")) {
                v["provider"] = json!("snapd");
                match snapd::installed(&tools.snap_socket, &name) {
                    Ok(details) => {
                        if details["type"].as_str().unwrap_or("app") != "app"
                            || ["snapd", "core", "core18", "core20", "core22", "core24"]
                                .contains(&name.as_str())
                        {
                            return Ok(blocked(
                                v,
                                lang,
                                "protected",
                                "Deze Snap hoort bij de systeembasis.",
                                "This Snap is part of the base system.",
                            ));
                        }
                        v["supported"] = json!(true);
                        v["_package"] = json!(name.as_str());
                        v["description"] = json!(t(
                            lang,
                            "Verwijdert de Snap-app. Snapd behoudt de standaard gegevens-snapshot.",
                            "Removes the Snap application. Snapd keeps its default data snapshot."
                        ));
                        return Ok(v);
                    }
                    Err(_) => {
                        return Ok(blocked(
                            v,
                            lang,
                            "provider_unavailable",
                            "Snapd kon deze geïnstalleerde app niet vinden.",
                            "Snapd could not resolve this installed application.",
                        ));
                    }
                }
            }
        }
    }
    if path.starts_with("/usr/share/applications")
        || path.starts_with("/usr/local/share/applications")
    {
        if let Some(pkg) = owner(&path, tools) {
            v["provider"] = json!("packagekit");
            let mut cmd = Command::new(&tools.dpkg_query);
            cmd.args(["--show","--showformat=${binary:Package}\t${Architecture}\t${Essential}\t${Priority}\t${db:Status-Status}\n",&pkg]);
            if let Ok(out) = bounded(cmd, Duration::from_secs(5), 8192) {
                let text = String::from_utf8_lossy(&out);
                let f = text.trim_end().split('\t').collect::<Vec<_>>();
                if f.len() == 5 && package(f[0]) && f[4] == "installed" {
                    if protected(f[0]) || f[2] == "yes" || f[3] == "required" {
                        return Ok(blocked(
                            v,
                            lang,
                            "protected",
                            "Dit pakket is essentieel voor het systeem en kan hier niet worden verwijderd.",
                            "This package is essential to the system and cannot be removed here.",
                        ));
                    }
                    v["supported"] = json!(true);
                    v["_package"] = json!(f[0]);
                    v["_architecture"] = json!(f[1]);
                    v["description"] = json!(format!(
                        "{} {}",
                        t(lang, "Verwijdert het pakket", "Removes package"),
                        f[0]
                    ));
                    return Ok(v);
                }
            }
        }
    }
    Ok(blocked(
        v,
        lang,
        "unsupported_provider",
        "De installatiebron is niet betrouwbaar vast te stellen. Gebruik de oorspronkelijke pakketbeheerder.",
        "The installation source could not be reliably identified. Use its original package manager.",
    ))
}
fn owner(path: &Path, tools: &Tools) -> Option<String> {
    let mut cmd = Command::new(&tools.dpkg_query);
    cmd.arg("--search").arg(path);
    let out = bounded(cmd, Duration::from_secs(5), 65536).ok()?;
    package_owner_output(&String::from_utf8_lossy(&out), path)
}
pub(super) fn package_owner_output(text: &str, path: &Path) -> Option<String> {
    let mut owners = text
        .lines()
        .filter_map(|line| {
            let (p, f) = line.split_once(": ")?;
            (f == path.to_str()? && package(p)).then(|| p.to_string())
        })
        .collect::<Vec<_>>();
    owners.sort();
    owners.dedup();
    (owners.len() == 1).then(|| owners.remove(0))
}

pub(super) fn valid_ref(s: &str) -> bool {
    let p = s.split('/').collect::<Vec<_>>();
    p.len() == 4
        && p[0] == "app"
        && flatpak_id(p[1])
        && p[2..].iter().all(|s| {
            !s.is_empty()
                && s.len() < 100
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
}
pub(super) fn remove_shortcut(info: &Value) -> Result<(), String> {
    let p = PathBuf::from(info["_launcher"].as_str().ok_or("Missing launcher")?);
    let m = std::fs::symlink_metadata(&p).map_err(|e| e.to_string())?;
    let old = &info["_identity"];
    if m.uid() != unsafe { libc::geteuid() }
        || json!({"dev":m.dev(),"ino":m.ino(),"mtime":m.mtime(),"mtimeNsec":m.mtime_nsec(),"size":m.len()})
            != *old
    {
        return Err("The launcher changed after review".into());
    }
    std::fs::remove_file(p).map_err(|e| e.to_string())
}
