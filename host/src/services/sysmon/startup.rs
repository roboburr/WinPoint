use super::common::*;
use crate::{
    files::{ApiError, ApiResult},
    i18n::Lang,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};
pub(super) fn parse(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut group = "";
    for line in text.lines() {
        let s = line.trim();
        if s.is_empty() || s.starts_with('#') {
            continue;
        }
        if s.starts_with('[') && s.ends_with(']') {
            group = &s[1..s.len() - 1];
            continue;
        }
        if group != "Desktop Entry" {
            continue;
        }
        if let Some((k, v)) = s.split_once('=') {
            out.entry(k.trim().into())
                .or_insert_with(|| v.trim().into());
        }
    }
    out
}
pub(super) fn local(d: &BTreeMap<String, String>, key: &str, lang: &str) -> String {
    [
        format!("{key}[{lang}]"),
        format!("{key}[{lang}_{}]", if lang == "nl" { "NL" } else { "US" }),
        key.into(),
    ]
    .iter()
    .find_map(|k| d.get(k).filter(|s| !s.is_empty()).cloned())
    .unwrap_or_default()
}
fn dirs() -> (PathBuf, Vec<PathBuf>) {
    (
        config_home().join("autostart"),
        std::env::var("XDG_CONFIG_DIRS")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "/etc/xdg".into())
            .split(':')
            .filter(|s| !s.is_empty())
            .map(|s| Path::new(s).join("autostart"))
            .collect(),
    )
}
fn shows(d: &BTreeMap<String, String>, desktops: &[String]) -> bool {
    let members = |key| {
        d.get(key)
            .into_iter()
            .flat_map(|s| s.split(';'))
            .filter(|s| !s.is_empty())
            .map(str::to_lowercase)
            .collect::<Vec<_>>()
    };
    let only = members("OnlyShowIn");
    let notin = members("NotShowIn");
    if !only.is_empty() && !only.iter().any(|s| desktops.contains(s)) {
        return false;
    }
    if notin.iter().any(|s| desktops.contains(s)) {
        return false;
    }
    d.get("TryExec").is_none_or(|s| which(s, None).is_some())
}
pub(super) fn list(lang: &str, include_hidden: bool) -> Value {
    let (user, system) = dirs();
    let desktops = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    let mut files = BTreeMap::new();
    for d in system.iter().rev() {
        for n in names(d) {
            if n.ends_with(".desktop") {
                files.insert(n.clone(), ("system", d.join(n)));
            }
        }
    }
    let system_files = files.clone();
    for n in names(&user) {
        if n.ends_with(".desktop") {
            files.insert(n.clone(), ("user", user.join(n)));
        }
    }
    let mut out = vec![];
    for (fid, (source, path)) in files {
        let d = parse(&read(&path));
        if d.is_empty() || d.get("Type").is_some_and(|s| s != "Application") {
            continue;
        }
        let mut merged = d.clone();
        if source == "user"
            && system_files.contains_key(&fid)
            && (d.get("Exec").is_none_or(|s| s.is_empty())
                || d.get("Name").is_none_or(|s| s.is_empty()))
        {
            merged = parse(&read(&system_files[&fid].1));
            merged.extend(d);
        }
        if !shows(&merged, &desktops) {
            continue;
        }
        let val = |k: &str| merged.get(k).map(String::as_str).unwrap_or("");
        let hidden = val("NoDisplay").eq_ignore_ascii_case("true")
            || !val("X-GNOME-Autostart-Phase").is_empty();
        if hidden && !include_hidden {
            continue;
        }
        if source == "user"
            && val("Hidden").eq_ignore_ascii_case("true")
            && !system_files.contains_key(&fid)
            && val("Exec").is_empty()
        {
            continue;
        }
        let name = local(&merged, "Name", lang);
        let description = local(&merged, "Comment", lang);
        out.push(json!({"id":fid,"name":if name.is_empty(){fid.trim_end_matches(".desktop").to_owned()}else{name},"icon":if val("Icon").is_empty(){"application-x-executable"}else{val("Icon")},"enabled":!val("Hidden").eq_ignore_ascii_case("true")&&!val("X-GNOME-Autostart-enabled").eq_ignore_ascii_case("false"),"command":val("Exec"),"source":if system_files.contains_key(&fid){"system"}else{"user"},"description":if description.is_empty(){Value::Null}else{json!(description)},"systemDefault":system_files.contains_key(&fid),"path":path,"hidden":hidden}));
    }
    out.sort_by_key(|v| v["name"].as_str().unwrap_or("").to_lowercase());
    json!(out)
}
pub(super) fn set_key(text: &str, key: &str, value: &str) -> String {
    let lines = text.lines().collect::<Vec<_>>();
    let mut start = None;
    let mut end = None;
    for (i, line) in lines.iter().enumerate() {
        let s = line.trim();
        if s.starts_with('[') && s.ends_with(']') {
            if start.is_some() && end.is_none() {
                end = Some(i)
            }
            if s == "[Desktop Entry]" && start.is_none() {
                start = Some(i)
            }
        }
    }
    let Some(start) = start else {
        return format!(
            "[Desktop Entry]\n{key}={value}\n{}",
            if text.trim().is_empty() {
                String::new()
            } else {
                format!("\n{}\n", text.trim_matches('\n'))
            }
        );
    };
    let end = end.unwrap_or(lines.len());
    let mut body = lines[start + 1..end]
        .iter()
        .filter(|l| l.split_once('=').is_none_or(|(k, _)| k.trim() != key))
        .copied()
        .collect::<Vec<_>>();
    while body.last().is_some_and(|s| s.trim().is_empty()) {
        body.pop();
    }
    let new = format!("{key}={value}");
    let mut out = lines[..start + 1].to_vec();
    out.extend(body);
    out.push(&new);
    if end < lines.len() {
        out.push("");
        out.extend(&lines[end..]);
    }
    format!("{}\n", out.join("\n").trim_end_matches('\n'))
}
pub(super) fn set(fid: &Value, enabled: bool, lang: Lang) -> ApiResult {
    let fid = fid
        .as_str()
        .filter(|s| {
            s.ends_with(".desktop")
                && s.len() >= 9
                && s.len() <= 208
                && s.trim_end_matches(".desktop").len() <= 200
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._@+-".contains(&b))
        })
        .ok_or_else(|| {
            ApiError::tr(
                "bad_args",
                lang,
                "sysmon.startup.unknown",
                crate::i18n::NO_PARAMS,
            )
        })?;
    let (user, sys) = dirs();
    let dst = user.join(fid);
    let src = std::iter::once(dst.clone())
        .chain(sys.into_iter().map(|p| p.join(fid)))
        .find(|p| p.is_file())
        .ok_or_else(|| {
            ApiError::tr(
                "not_found",
                lang,
                "sysmon.startup.gone",
                crate::i18n::NO_PARAMS,
            )
        })?;
    let data = fs::read(&src).map_err(|_| {
        ApiError::tr(
            "failed",
            lang,
            "sysmon.startup.readFailed",
            crate::i18n::NO_PARAMS,
        )
    })?;
    let text = String::from_utf8_lossy(&data);
    let text = set_key(
        &set_key(&text, "Hidden", if enabled { "false" } else { "true" }),
        "X-GNOME-Autostart-enabled",
        if enabled { "true" } else { "false" },
    );
    let write = || -> std::io::Result<()> {
        fs::create_dir_all(&user)?;
        let temp = user.join(format!(".{fid}.{}.new", crate::events::nonce()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o644)
            .open(&temp)?;
        let result = (|| {
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            fs::rename(&temp, &dst)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    };
    write().map_err(|_| {
        ApiError::tr(
            "failed",
            lang,
            "sysmon.startup.saveFailed",
            crate::i18n::NO_PARAMS,
        )
    })?;
    Ok(json!({"id":fid,"enabled":enabled}))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keys_preserve_other_groups_and_remove_duplicates() {
        let text = "[Desktop Entry]\nName=X\n Hidden=true\nHidden=false\n\n[Other]\nHidden=keep\n";
        assert_eq!(
            set_key(text, "Hidden", "true"),
            "[Desktop Entry]\nName=X\nHidden=true\n\n[Other]\nHidden=keep\n"
        );
    }
    #[test]
    fn translations_follow_language_not_process_locale() {
        let d = parse(
            "[Desktop Entry]\nName=Base\nName[nl]=Nederlands\nName[en_US]=English\nName=Second\n",
        );
        assert_eq!(local(&d, "Name", "nl"), "Nederlands");
        assert_eq!(local(&d, "Name", "en"), "English");
    }
    #[test]
    fn invalid_id_is_rejected_before_any_file_io() {
        for s in ["../bad.desktop", "-x/no.desktop", "a\n.desktop", ""] {
            assert_eq!(set(&json!(s), true, Lang::Nl).unwrap_err().code, "bad_args");
        }
    }
}
