use super::{Tools, common::*};
use crate::files::ApiResult;
use serde_json::{Value, json};
use std::{
    io::{Read, Seek, SeekFrom},
    path::Path,
    process::Command,
    time::Duration,
};
fn extension(path: &Path) -> String {
    path.extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase()
}
pub(super) fn inspect(raw: &Value, lang: &str, tools: &Tools) -> ApiResult {
    let (path, mut f, identity) = source(raw, lang)?;
    let mut header = [0u8; 4096];
    let n = f.read(&mut header).map_err(|_| {
        err(
            lang,
            "no_access",
            "Dit bestand kan niet worden gelezen.",
            "This file cannot be read.",
        )
    })?;
    f.seek(SeekFrom::Start(0)).map_err(|_| {
        err(
            lang,
            "no_access",
            "Dit bestand kan niet worden gelezen.",
            "This file cannot be read.",
        )
    })?;
    let h = &header[..n];
    let ext = extension(&path);
    let kind = if h.starts_with(b"\x7fELF")
        && h.get(8..10) == Some(b"AI")
        && h.get(10).is_some_and(|v| [1, 2].contains(v))
    {
        "appimage"
    } else if ext == "deb" && h.starts_with(b"!<arch>\n") {
        "deb"
    } else if ext == "snap" && (h.starts_with(b"hsqs") || h.starts_with(b"sqsh")) {
        "snap"
    } else if ext == "flatpakref" {
        "flatpakref"
    } else if ext == "flatpak" && h.len() >= 32 && h.contains(&0) && !h.starts_with(b"\x7fELF") {
        "flatpak"
    } else {
        return Err(err(
            lang,
            "unsupported",
            "Dit bestand is geen ondersteund installatiepakket.",
            "This file is not a supported installation package.",
        ));
    };
    let fallback = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let (mut details, provider, scope) = match kind {
        "appimage" => {
            let machine = if h.get(5) == Some(&2) {
                h.get(18..20).map(|s| u16::from_be_bytes([s[0], s[1]]))
            } else {
                h.get(18..20).map(|s| u16::from_le_bytes([s[0], s[1]]))
            };
            let arch = match machine {
                Some(62) => "x86_64",
                Some(183) => "aarch64",
                Some(3) => "i386",
                Some(40) => "arm",
                _ => "unknown",
            };
            (
                json!({"name":fallback,"version":"","architecture":arch,"description":t(lang,"Een zelfstandige AppImage-app. Er wordt een beheerde kopie met een snelkoppeling geïnstalleerd.","A standalone AppImage application. A managed copy and launcher will be installed."),"notes":[t(lang,"Het gedownloade bestand blijft staan.","The downloaded file will be kept.")]}),
                "appimage",
                "user",
            )
        }
        "flatpakref" => (flatpak_ref(&path, lang)?, "flatpak", "user"),
        "flatpak" => (
            json!({"name":fallback,"version":"","architecture":"","description":t(lang,"Lokaal Flatpak-app-pakket","Local Flatpak application bundle"),"notes":[t(lang,"Flatpak controleert het pakket en de handtekening tijdens de installatie.","Flatpak checks the bundle and its signature during installation.")]}),
            "flatpak",
            "user",
        ),
        "deb" => (
            read_worker(kind, &path, tools, lang)?,
            "packagekit",
            "system",
        ),
        "snap" => (read_worker(kind, &path, tools, lang)?, "snapd", "system"),
        _ => unreachable!(),
    };
    let supported = match provider {
        "packagekit" => tools.dpkg_query.is_file() && super::packagekit::available(),
        "flatpak" => tools.flatpak.is_file(),
        "snapd" => tools.snap_socket.exists(),
        _ => true,
    };
    let protected = kind == "deb"
        && (details["essential"] == true
            || details["priority"] == "required"
            || details["package"].as_str().is_some_and(protected))
        || kind == "snap" && details["type"].as_str().is_some_and(|s| s != "app");
    if kind == "snap" {
        details["notes"] = json!([
            t(
                lang,
                "Een lokaal Snap-pakket kan zonder winkelhandtekening worden geïnstalleerd. Installeer het alleen als je de bron vertrouwt.",
                "A local Snap package can be installed without a store signature. Install it only if you trust the source."
            ),
            if details["confinement"] == "classic" {
                t(
                    lang,
                    "Deze app vraagt volledige systeemtoegang (classic).",
                    "This application requests full system access (classic).",
                )
            } else if details["confinement"] == "devmode" {
                t(
                    lang,
                    "De beveiligingsisolatie is uitgeschakeld voor dit pakket (devmode).",
                    "Security confinement is disabled for this package (devmode).",
                )
            } else {
                t(
                    lang,
                    "Dit pakket gebruikt strikte Snap-isolatie.",
                    "This package uses strict Snap confinement.",
                )
            }
        ]);
    }
    details["format"] = json!(kind);
    details["provider"] = json!(provider);
    details["scope"] = json!(scope);
    details["size"] = identity["size"].clone();
    details["sourcePath"] = json!(path);
    details["supported"] = json!(supported && !protected);
    if protected {
        details["reasonCode"] = json!("protected");
        details["reason"] = json!(t(
            lang,
            "Dit pakket hoort bij de systeembasis en kan hier niet worden gewijzigd.",
            "This package is part of the base system and cannot be changed here."
        ));
    } else if !supported {
        details["reasonCode"] = json!("provider_missing");
        details["reason"] = json!(t(
            lang,
            "De pakketbeheerder voor dit bestand is niet geïnstalleerd.",
            "The package manager for this file is not installed."
        ));
    }
    details["_identity"] = identity;
    Ok(details)
}
fn flatpak_ref(path: &Path, lang: &str) -> ApiResult {
    let m = path.metadata().map_err(|_| {
        err(
            lang,
            "no_access",
            "Dit bestand kan niet worden gelezen.",
            "This file cannot be read.",
        )
    })?;
    if m.len() > 1 << 20 {
        return Err(err(
            lang,
            "unsupported",
            "Dit Flatpak-verwijzingsbestand is te groot.",
            "This Flatpak reference file is too large.",
        ));
    }
    let key = glib::KeyFile::new();
    key.load_from_file(path, glib::KeyFileFlags::NONE)
        .map_err(|_| {
            err(
                lang,
                "invalid_package",
                "Dit Flatpak-bestand is beschadigd.",
                "This Flatpak file is damaged.",
            )
        })?;
    let id = key
        .string("Flatpak Ref", "Name")
        .ok()
        .filter(|s| flatpak_id(s))
        .ok_or_else(|| {
            err(
                lang,
                "invalid_package",
                "Dit Flatpak-bestand bevat geen geldige app.",
                "This Flatpak file does not identify a valid application.",
            )
        })?;
    if key.boolean("Flatpak Ref", "IsRuntime").unwrap_or(false) {
        return Err(err(
            lang,
            "unsupported",
            "Dit bestand bevat een runtime, geen app.",
            "This file describes a runtime rather than an application.",
        ));
    }
    let url = key
        .string("Flatpak Ref", "Url")
        .ok()
        .and_then(|s| url::Url::parse(&s).ok())
        .filter(|u| matches!(u.scheme(), "https" | "http" | "file"))
        .ok_or_else(|| {
            err(
                lang,
                "invalid_package",
                "Dit Flatpak-bestand bevat geen geldige bron.",
                "This Flatpak file has no valid source.",
            )
        })?;
    let name = key
        .string("Flatpak Ref", "Title")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|s| clean(&s, 200))
        .unwrap_or_else(|| id.to_string());
    Ok(
        json!({"name":name,"version":key.string("Flatpak Ref","Branch").ok().map(|s|clean(&s,100)).unwrap_or_default(),"architecture":"","description":key.string("Flatpak Ref","Comment").ok().map(|s|clean(&s,2000)).unwrap_or_default(),"appId":id.as_str(),"origin":url.to_string(),"notes":[]}),
    )
}
fn read_worker(kind: &str, path: &Path, tools: &Tools, lang: &str) -> ApiResult {
    let _slot = super::worker::metadata_slot();
    let mut c = super::worker::command(&tools.worker, path);
    c.arg("--inspect").arg(kind).arg(path);
    let output = bounded(c, Duration::from_secs(12), 1 << 20).map_err(|_| {
        err(
            lang,
            "invalid_package",
            "De pakketgegevens konden niet veilig worden gelezen.",
            "The package details could not be read safely.",
        )
    })?;
    let result: Value = serde_json::from_slice(&output).map_err(|_| {
        err(
            lang,
            "invalid_package",
            "De pakketgegevens zijn ongeldig.",
            "The package details are invalid.",
        )
    })?;
    if result["ok"] == true {
        Ok(result["details"].clone())
    } else {
        Err(err(
            lang,
            "invalid_package",
            "Dit installatiepakket is beschadigd of niet ondersteund.",
            "This installation package is damaged or unsupported.",
        ))
    }
}
pub(super) fn worker_inspect(kind: &str, path: &Path, tools: &Tools) -> Result<Value, String> {
    match kind {
        "deb" => {
            let mut cmd = Command::new(&tools.dpkg_deb);
            cmd.arg("--field").arg(path).args([
                "Package",
                "Version",
                "Architecture",
                "Description",
                "Installed-Size",
                "Essential",
                "Priority",
            ]);
            let bytes = bounded(cmd, Duration::from_secs(6), 1 << 20).map_err(|e| e.to_string())?;
            let text = String::from_utf8_lossy(&bytes);
            let mut fields = std::collections::HashMap::<String, String>::new();
            let mut last = String::new();
            for line in text.lines() {
                if line.starts_with(' ') || line.starts_with('\t') {
                    if let Some(s) = fields.get_mut(&last) {
                        s.push('\n');
                        s.push_str(line.trim());
                    }
                } else if let Some((k, v)) = line.split_once(':') {
                    last = k.into();
                    fields.insert(k.into(), v.trim().into());
                }
            }
            let package_name = fields
                .get("Package")
                .filter(|s| package(s))
                .ok_or("Invalid package name")?;
            let desc = fields.get("Description").cloned().unwrap_or_default();
            Ok(
                json!({"name":clean(desc.lines().next().filter(|s|!s.is_empty()).unwrap_or(package_name),200),"package":package_name,"version":fields.get("Version").map(|s|clean(s,200)).unwrap_or_default(),"architecture":fields.get("Architecture").map(|s|clean(s,100)).unwrap_or_default(),"description":clean(&desc,4000),"installedSize":fields.get("Installed-Size").and_then(|s|s.parse::<u64>().ok()).map(|n|n.saturating_mul(1024)),"essential":fields.get("Essential").is_some_and(|s|s=="yes"),"priority":fields.get("Priority"),"notes":[]}),
            )
        }
        "snap" => {
            let mut cmd = Command::new(&tools.unsquashfs);
            cmd.args(["-no-progress", "-processors", "1", "-cat"])
                .arg(path)
                .arg("meta/snap.yaml");
            let bytes = bounded(cmd, Duration::from_secs(6), 1 << 20).map_err(|e| e.to_string())?;
            let text = String::from_utf8_lossy(&bytes);
            let mut fields = std::collections::HashMap::new();
            for line in text
                .lines()
                .filter(|l| !l.starts_with(char::is_whitespace) && !l.starts_with('#'))
            {
                if let Some((k, v)) = line.split_once(':') {
                    fields.insert(k.trim(), v.trim().trim_matches(['\'', '"']));
                }
            }
            let name = fields
                .get("name")
                .filter(|s| snap_name(s))
                .ok_or("Invalid snap name")?;
            let confinement = fields.get("confinement").copied().unwrap_or("strict");
            if !["strict", "classic", "devmode"].contains(&confinement) {
                return Err("Invalid confinement".into());
            }
            Ok(
                json!({"name":name,"snapName":name,"version":fields.get("version").map(|s|clean(s,200)).unwrap_or_default(),"architecture":"","description":fields.get("summary").map(|s|clean(s,2000)).unwrap_or_default(),"type":fields.get("type").copied().unwrap_or("app"),"confinement":confinement,"notes":[]}),
            )
        }
        _ => Err("Unsupported metadata format".into()),
    }
}
