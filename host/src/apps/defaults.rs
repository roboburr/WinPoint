//! Native Open With/default associations. Mutations use GIO plus a shared config lock.
use super::catalog::{Catalog, compare_names, own_icon};
use crate::files::{ApiError, ApiResult, actions};
use gio::prelude::*;
use glib::translate::ToGlibPtr;
use serde_json::{Map, Value, json};
use std::{
    collections::HashSet,
    ffi::{CString, OsStr},
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    ptr,
    sync::atomic::{AtomicU64, Ordering},
};
const OWN_FILES: &str = "win11shell-explorer.desktop";
#[derive(Debug)]
struct Spec {
    kind: &'static str,
    main: Option<&'static str>,
    types: &'static [&'static str],
}
const SPECS: &[Spec] = &[
    Spec {
        kind: "browser",
        main: Some("x-scheme-handler/https"),
        types: &[
            "x-scheme-handler/http",
            "x-scheme-handler/https",
            "text/html",
            "application/xhtml+xml",
        ],
    },
    Spec {
        kind: "mail",
        main: Some("x-scheme-handler/mailto"),
        types: &["x-scheme-handler/mailto"],
    },
    Spec {
        kind: "files",
        main: Some("inode/directory"),
        types: &["inode/directory", "application/x-gnome-saved-search"],
    },
    Spec {
        kind: "music",
        main: Some("audio/mpeg"),
        types: &[
            "audio/mpeg",
            "audio/mp4",
            "audio/flac",
            "audio/x-flac",
            "audio/ogg",
            "audio/x-vorbis+ogg",
            "audio/vnd.wave",
            "audio/x-wav",
            "audio/aac",
            "audio/opus",
            "audio/x-opus+ogg",
            "audio/webm",
            "audio/x-m4a",
            "audio/x-mpegurl",
        ],
    },
    Spec {
        kind: "video",
        main: Some("video/mp4"),
        types: &[
            "video/mp4",
            "video/x-matroska",
            "video/webm",
            "video/quicktime",
            "video/x-msvideo",
            "video/mpeg",
            "video/ogg",
            "video/x-flv",
            "video/3gpp",
        ],
    },
    Spec {
        kind: "image",
        main: Some("image/png"),
        types: &[
            "image/png",
            "image/jpeg",
            "image/gif",
            "image/webp",
            "image/bmp",
            "image/tiff",
            "image/svg+xml",
            "image/avif",
            "image/heif",
            "image/x-icon",
            "image/jxl",
        ],
    },
    Spec {
        kind: "text",
        main: Some("text/plain"),
        types: &["text/plain", "text/markdown", "text/x-log"],
    },
    Spec {
        kind: "terminal",
        main: None,
        types: &[],
    },
];

pub(super) fn call(method: &str, args: &Value, catalog: &Catalog, lang: &str) -> Option<ApiResult> {
    match method {
        "apps.forFile" => Some(for_file(args, catalog, lang)),
        "apps.defaults" => Some(defaults(args, catalog, lang)),
        "apps.setDefault" => Some(set_default(args, lang)),
        _ => None,
    }
}
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(v) => *v,
        Value::Number(v) => v.as_f64() != Some(0.0),
        Value::String(v) => !v.is_empty(),
        Value::Array(v) => !v.is_empty(),
        Value::Object(v) => !v.is_empty(),
    }
}
fn desktop_info(id: &str) -> Option<gio::DesktopAppInfo> {
    if id.contains('\0') {
        None
    } else {
        gio::DesktopAppInfo::new(id)
    }
}
fn content_type(path: &Path) -> String {
    if path.is_dir() {
        return "inode/directory".into();
    }
    if let Ok(info) = gio::File::for_path(path).query_info(
        "standard::content-type",
        gio::FileQueryInfoFlags::NONE,
        gio::Cancellable::NONE,
    ) {
        if let Some(content) = info.content_type().filter(|content| !content.is_empty()) {
            return content.to_string();
        }
    }
    // _guess_type uses filename-only data=NULL, not a present zero-byte data buffer.
    crate::files::filename_mime(&path.to_string_lossy())
}
fn for_file(args: &Value, catalog: &Catalog, lang: &str) -> ApiResult {
    let path = args["path"]
        .as_str()
        .filter(|path| Path::new(path).is_absolute())
        .ok_or_else(|| ApiError::tr("bad_args", lang, "apps.invalidPath", crate::i18n::NO_PARAMS))?;
    let path = Path::new(path);
    if !path.exists() {
        return Err(ApiError::tr("not_found", lang, "apps.fileGone", crate::i18n::NO_PARAMS));
    }
    let content = content_type(path);
    let directory = content == "inode/directory";
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    if directory {
        result.push(json!({"id":OWN_FILES,"name":crate::i18n::tr(lang, "apps.explorerName"),"icon":own_icon("explorer"),"isDefault":true}));
        seen.insert(OWN_FILES.to_owned());
    }
    let default = gio::AppInfo::default_for_type(&content, false);
    let default_id = default
        .as_ref()
        .and_then(|info| info.id())
        .map(|id| id.to_string());
    let mut ordered = Vec::new();
    if let Some(default) = default {
        ordered.push(default);
    }
    ordered.extend(
        gio::AppInfo::all_for_type(&content)
            .into_iter()
            .filter(|info| info.id().as_deref() != default_id.as_deref()),
    );
    for info in ordered {
        let Some(id) = info
            .id()
            .filter(|id| !id.is_empty())
            .map(|id| id.to_string())
        else {
            continue;
        };
        if seen.contains(&id) || (Some(id.as_str()) != default_id.as_deref() && !info.should_show())
        {
            continue;
        }
        seen.insert(id.clone());
        let mut brief = catalog
            .brief(&info, lang)
            .unwrap_or_else(|| json!({"id":id,"name":info.name().as_str(),"icon":""}));
        brief["isDefault"] = json!(Some(id.as_str()) == default_id.as_deref() && !directory);
        result.push(brief);
    }
    Ok(json!(result))
}
fn default_for(spec: &Spec) -> Option<gio::AppInfo> {
    match spec.kind {
        "terminal" => actions::default_terminal().map(|info| info.upcast()),
        "browser" => gio::AppInfo::default_for_uri_scheme("https")
            .or_else(|| gio::AppInfo::default_for_uri_scheme("http"))
            .or_else(|| gio::AppInfo::default_for_type("text/html", false)),
        "mail" => gio::AppInfo::default_for_uri_scheme("mailto"),
        _ => spec
            .main
            .into_iter()
            .chain(spec.types.iter().copied())
            .find_map(|content| gio::AppInfo::default_for_type(content, false)),
    }
}
fn candidates(spec: &Spec) -> Vec<gio::AppInfo> {
    if spec.kind == "terminal" {
        return gio::AppInfo::all()
            .into_iter()
            .filter(|info| {
                info.downcast_ref::<gio::DesktopAppInfo>()
                    .and_then(|info| info.categories())
                    .is_some_and(|categories| {
                        categories
                            .split(';')
                            .any(|category| category == "TerminalEmulator")
                    })
            })
            .collect();
    }
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for content in spec
        .main
        .into_iter()
        .chain(spec.types.iter().take(3).copied())
    {
        for info in gio::AppInfo::all_for_type(content) {
            let id = info.id().map(|id| id.to_string());
            if !seen.contains(&id) && info.should_show() {
                seen.insert(id);
                result.push(info);
            }
        }
    }
    if spec.kind == "files" && !seen.contains(&Some(OWN_FILES.to_owned())) {
        if let Some(own) = desktop_info(OWN_FILES) {
            result.insert(0, own.upcast());
        }
    }
    result
}
fn defaults(args: &Value, catalog: &Catalog, lang: &str) -> ApiResult {
    let mut result = Map::new();
    for spec in SPECS {
        result.insert(
            spec.kind.into(),
            default_for(spec)
                .and_then(|info| catalog.brief(&info, lang))
                .unwrap_or(Value::Null),
        );
    }
    if truthy(&args["candidates"]) {
        let mut lists = Map::new();
        for spec in SPECS {
            let mut items: Vec<_> = candidates(spec)
                .iter()
                .filter_map(|info| catalog.brief(info, lang))
                .collect();
            items.sort_by(|first, second| {
                compare_names(
                    first["name"].as_str().unwrap_or(""),
                    second["name"].as_str().unwrap_or(""),
                )
            });
            lists.insert(spec.kind.into(), json!(items));
        }
        result.insert("candidates".into(), Value::Object(lists));
    }
    Ok(Value::Object(result))
}
fn own_app(id: &str) -> Option<&'static str> {
    let folded = id.trim().to_lowercase();
    let folded = folded.strip_suffix(".desktop").unwrap_or(&folded);
    let folded = folded.strip_prefix("win11shell-").unwrap_or(folded);
    crate::launch::OWN_APPS
        .iter()
        .copied()
        .find(|app| *app == folded)
}
fn selected_default(
    args: &Value,
    lang: &str,
) -> Result<(&'static Spec, String, bool), ApiError> {
    let kind = args["kind"]
        .as_str()
        .and_then(|kind| SPECS.iter().find(|spec| spec.kind == kind))
        .ok_or_else(|| ApiError::tr("bad_args", lang, "apps.unknownDefaultKind", crate::i18n::NO_PARAMS))?;
    let id = args["id"]
        .as_str()
        .filter(|id| !id.is_empty() && id.chars().count() <= 300 && !id.contains('/'))
        .ok_or_else(|| ApiError::tr("bad_args", lang, "apps.invalidApp", crate::i18n::NO_PARAMS))?;
    if let Some(own) = own_app(id) {
        if kind.kind != "files" || own != "explorer" {
            return Err(ApiError::tr("bad_args", lang, "apps.cantBeDefault", crate::i18n::NO_PARAMS));
        }
        return Ok((kind, format!("win11shell-{own}.desktop"), true));
    }
    Ok((kind, id.into(), false))
}
fn should_set(spec: &Spec, content: &str, supported: &HashSet<String>) -> bool {
    Some(content) == spec.main
        || spec.kind == "browser"
        || supported.is_empty()
        || supported.contains(content)
}
fn internal(lang: &str) -> ApiError {
    ApiError::tr("internal", lang, "apps.somethingWentWrong", crate::i18n::NO_PARAMS)
}
fn set_default(args: &Value, lang: &str) -> ApiResult {
    let (spec, id, own) = selected_default(args, lang)?;
    let info=desktop_info(&id).ok_or_else(||if own {
        let mut error=ApiError::tr("not_installed", lang, "apps.explorerNotInstalled", crate::i18n::NO_PARAMS);
        error.data=Some(json!({"id":id}));error
    } else {ApiError::tr("not_found", lang, "apps.appNotFound", crate::i18n::NO_PARAMS)})?;
    let config = glib::user_config_dir();
    let _lock = ConfigLock::acquire(&config).map_err(|_| {
        if spec.kind == "terminal" {
            internal(lang)
        } else {
            ApiError::tr("failed", lang, "apps.defaultNotChanged", crate::i18n::NO_PARAMS)
        }
    })?;
    if spec.kind == "terminal" {
        set_terminal(&config.join("xdg-terminals.list"), &id).map_err(|_| internal(lang))?;
        return Ok(json!({"types":["terminal"]}));
    }
    let supported: HashSet<_> = info
        .supported_types()
        .iter()
        .map(|content| content.to_string())
        .collect();
    let (mut done, mut skipped) = (Vec::new(), Vec::new());
    for content in spec.types {
        if !should_set(spec, content, &supported) {
            skipped.push(*content);
            continue;
        }
        if info.set_as_default_for_type(content).is_ok() {
            done.push(*content);
        }
    }
    if done.is_empty() {
        return Err(ApiError::tr("failed", lang, "apps.defaultNotChanged", crate::i18n::NO_PARAMS));
    }
    let mut cleared = Vec::new();
    if spec.kind == "files" && id != OWN_FILES {
        for content in skipped {
            if drop_default_entry(&config.join("mimeapps.list"), content, OWN_FILES)
                .unwrap_or(false)
            {
                cleared.push(content);
            }
        }
    }
    let mut result = json!({"types":done,"effective":true,"cleared":cleared});
    if let Some(now) = spec
        .main
        .and_then(|content| gio::AppInfo::default_for_type(content, false))
    {
        if now.id().as_deref() != Some(id.as_str()) {
            result["effective"] = json!(false);
            result["overriddenBy"] = json!(desktop_mimeapps(
                &config,
                &std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default()
            ));
            result["current"] = json!(now.id().as_deref());
        }
    }
    Ok(result)
}
pub(super) fn desktop_mimeapps(config: &Path, desktops: &str) -> Vec<PathBuf> {
    desktops
        .split(':')
        .filter(|desktop| !desktop.is_empty())
        .map(|desktop| config.join(format!("{}-mimeapps.list", desktop.to_lowercase())))
        .filter(|path| path.is_file())
        .collect()
}

pub(super) struct ConfigLock(File);
impl ConfigLock {
    pub(super) fn acquire(config: &Path) -> io::Result<Self> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(config)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(config.join(".apps-defaults.lock"))?;
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::getuid() }
            || metadata.nlink() != 1
            || metadata.mode() & 0o077 != 0
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Het standaard-app-slot moet privé zijn.",
            ));
        }
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
                return Ok(Self(file));
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
}
impl Drop for ConfigLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
fn atomic_text(path: &Path, text: &str) -> io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)?;
    let name = path
        .file_name()
        .unwrap_or(OsStr::new("associations"))
        .to_string_lossy();
    let (temporary, mut file) = loop {
        let temporary = parent.join(format!(
            ".{name}.{}.{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temporary)
        {
            Ok(file) => break (temporary, file),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };
    let result = (|| {
        file.write_all(text.as_bytes())?;
        file.set_permissions(fs::Permissions::from_mode(0o644))?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
fn terminal_contents(text: &str, id: &str) -> String {
    let lines = text
        .split([
            '\n', '\r', '\u{000b}', '\u{000c}', '\u{001c}', '\u{001d}', '\u{001e}', '\u{0085}',
            '\u{2028}', '\u{2029}',
        ])
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty()
                && trimmed
                    .trim_start_matches('+')
                    .split(':')
                    .next()
                    .unwrap_or("")
                    != id
        });
    std::iter::once(id)
        .chain(lines)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}
fn set_terminal(path: &Path, id: &str) -> io::Result<()> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::InvalidData => return Err(error),
        Err(_) => String::new(),
    };
    atomic_text(path, &terminal_contents(&text, id))
}
fn drop_default_entry(path: &Path, content: &str, id: &str) -> io::Result<bool> {
    let keyfile = glib::KeyFile::new();
    if keyfile
        .load_from_file(
            path,
            glib::KeyFileFlags::KEEP_COMMENTS | glib::KeyFileFlags::KEEP_TRANSLATIONS,
        )
        .is_err()
    {
        return Ok(false);
    }
    let mut changed = false;
    for group in ["Default Applications", "Added Associations"] {
        let Ok(values) = keyfile.string_list(group, content) else {
            continue;
        };
        let keep: Vec<_> = values
            .iter()
            .map(|value| value.to_string())
            .filter(|value| value != id)
            .collect();
        if keep.len() == values.len() {
            continue;
        }
        changed = true;
        if keep.is_empty() {
            let _ = keyfile.remove_key(group, content);
        } else {
            set_string_list(&keyfile, group, content, &keep)?;
        }
    }
    if changed {
        let mut text = keyfile.to_data().to_string();
        if !text.ends_with('\n') {
            text.push('\n');
        }
        atomic_text(path, &text)?;
    }
    Ok(changed)
}
fn set_string_list(
    keyfile: &glib::KeyFile,
    group: &str,
    key: &str,
    values: &[String],
) -> io::Result<()> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "Een app-id bevat een nulbyte.");
    let group = CString::new(group).map_err(|_| invalid())?;
    let key = CString::new(key).map_err(|_| invalid())?;
    let strings: Vec<_> = values
        .iter()
        .map(|value| CString::new(value.as_bytes()).map_err(|_| invalid()))
        .collect::<io::Result<_>>()?;
    let mut pointers: Vec<_> = strings.iter().map(|value| value.as_ptr()).collect();
    pointers.push(ptr::null());
    // glib's Rust 0.18 wrapper omits this public method. The list stays owned through the C call.
    unsafe {
        glib::ffi::g_key_file_set_string_list(
            keyfile.to_glib_none().0,
            group.as_ptr(),
            key.as_ptr(),
            pointers.as_ptr(),
            strings.len(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{os::unix::fs::symlink, sync::Arc};
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static ID: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "win11-defaults-{}-{}",
                std::process::id(),
                ID.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn selection_validation_and_own_explorer_aliases_preserve_reference() {
        assert_eq!(
            selected_default(&json!({"kind":"missing","id":"app.desktop"}), "nl")
                .unwrap_err()
                .message,
            "Onbekend soort standaard-app."
        );
        for id in [
            json!(""),
            json!("/tmp/app.desktop"),
            json!(false),
            json!("🦀".repeat(301)),
        ] {
            assert_eq!(
                selected_default(&json!({"kind":"text","id":id}), "nl")
                    .unwrap_err()
                    .message,
                "Ongeldige app."
            );
        }
        for id in [
            "explorer",
            "win11shell-explorer",
            "EXPLORER.desktop",
            " win11shell-explorer.desktop ",
        ] {
            let (spec, canonical, own) =
                selected_default(&json!({"kind":"files","id":id}), "nl").unwrap();
            assert_eq!(spec.kind, "files");
            assert_eq!(canonical, OWN_FILES);
            assert!(own);
        }
        for (kind, id) in [
            ("text", "explorer"),
            ("files", "settings"),
            ("terminal", "taskmgr"),
        ] {
            assert_eq!(
                selected_default(&json!({"kind":kind,"id":id}), "nl")
                    .unwrap_err()
                    .message,
                "Deze app kan hier niet als standaard worden gekozen."
            );
        }
        let (_, id, own) = selected_default(&json!({"kind":"text","id":"other.desktop"}), "nl").unwrap();
        assert_eq!(id, "other.desktop");
        assert!(!own);
        assert!(desktop_info("invalid\0.desktop").is_none());
    }
    #[test]
    fn secondary_mime_types_skip_only_when_declared_unsupported() {
        let text = SPECS.iter().find(|spec| spec.kind == "text").unwrap();
        let browser = SPECS.iter().find(|spec| spec.kind == "browser").unwrap();
        let files = SPECS.iter().find(|spec| spec.kind == "files").unwrap();
        let supported = HashSet::from(["text/plain".to_owned()]);
        assert!(should_set(text, "text/plain", &supported));
        assert!(!should_set(text, "text/markdown", &supported));
        assert!(should_set(text, "text/markdown", &HashSet::new()));
        assert!(should_set(browser, "application/xhtml+xml", &supported));
        assert!(should_set(files, "inode/directory", &supported));
        assert!(!should_set(
            files,
            "application/x-gnome-saved-search",
            &supported
        ));
        assert_eq!(
            SPECS.iter().map(|spec| spec.kind).collect::<Vec<_>>(),
            [
                "browser", "mail", "files", "music", "video", "image", "text", "terminal"
            ]
        );
    }
    #[test]
    fn terminal_write_prepends_selection_preserves_comments_order_and_disabled_entries() {
        let original = "# comment\r\n\r\n +chosen.desktop:arg\nother.desktop\rchosen.desktop\n -chosen.desktop\n \n  # spaced comment\n++chosen.desktop:second\nlast.desktop\u{2028}unicode.desktop";
        assert_eq!(
            terminal_contents(original, "chosen.desktop"),
            "chosen.desktop\n# comment\nother.desktop\n -chosen.desktop\n  # spaced comment\nlast.desktop\nunicode.desktop\n"
        );
    }
    #[test]
    fn secondary_explorer_cleanup_preserves_other_associations_comments_and_translations() {
        let temp = Temp::new();
        let path = temp.0.join("mimeapps.list");
        let content = "application/x-gnome-saved-search";
        fs::write(&path,format!("# Keep comment\n[Default Applications]\n{content}={OWN_FILES};other.desktop;{OWN_FILES};\ntext/plain=editor.desktop;\n\n[Added Associations]\n{content}=other\\;odd.desktop;{OWN_FILES};third.desktop;\n\n[Unrelated]\nName[nl]=Vertaling\n")).unwrap();
        assert!(drop_default_entry(&path, content, OWN_FILES).unwrap());
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("# Keep comment"));
        assert!(text.contains("Name[nl]=Vertaling"));
        let keyfile = glib::KeyFile::new();
        keyfile
            .load_from_file(&path, glib::KeyFileFlags::NONE)
            .unwrap();
        assert_eq!(
            keyfile
                .string_list("Default Applications", content)
                .unwrap()
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>(),
            ["other.desktop"]
        );
        assert_eq!(
            keyfile
                .string_list("Added Associations", content)
                .unwrap()
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>(),
            ["other;odd.desktop", "third.desktop"]
        );
        assert_eq!(
            keyfile
                .string_list("Default Applications", "text/plain")
                .unwrap()[0]
                .to_string(),
            "editor.desktop"
        );
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o644);
        assert!(!drop_default_entry(&path, content, OWN_FILES).unwrap());
    }
    #[test]
    fn cleanup_removes_empty_keys_and_leaves_untouched_files_byte_exact() {
        let temp = Temp::new();
        let path = temp.0.join("mimeapps.list");
        let content = "application/x-gnome-saved-search";
        let original = format!(
            "# original\n[Default Applications]\n{content}={OWN_FILES};\n[Added Associations]\n{content}={OWN_FILES};\n"
        );
        fs::write(&path, &original).unwrap();
        assert!(!drop_default_entry(&path, "text/plain", OWN_FILES).unwrap());
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        assert!(drop_default_entry(&path, content, OWN_FILES).unwrap());
        let keyfile = glib::KeyFile::new();
        keyfile
            .load_from_file(&path, glib::KeyFileFlags::NONE)
            .unwrap();
        assert!(
            keyfile
                .string_list("Default Applications", content)
                .is_err()
        );
        assert!(keyfile.string_list("Added Associations", content).is_err());
        fs::write(&path, b"invalid keyfile").unwrap();
        assert!(!drop_default_entry(&path, content, OWN_FILES).unwrap());
        assert_eq!(fs::read(&path).unwrap(), b"invalid keyfile");
    }
    #[test]
    fn atomic_terminal_writes_have_exact_modes_and_replace_only_the_requested_link() {
        let temp = Temp::new();
        let target = temp.0.join("original.list");
        let path = temp.0.join("xdg-terminals.list");
        fs::write(&target, "old.desktop\n").unwrap();
        symlink("original.list", &path).unwrap();
        set_terminal(&path, "chosen.desktop").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "old.desktop\n");
        assert!(!path.is_symlink());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "chosen.desktop\nold.desktop\n"
        );
        assert_eq!(path.metadata().unwrap().mode() & 0o777, 0o644);
        let directory = temp.0.join("cannot-replace");
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("sentinel"), b"preserved").unwrap();
        assert!(atomic_text(&directory, "new").is_err());
        assert_eq!(fs::read(directory.join("sentinel")).unwrap(), b"preserved");
        assert!(fs::read_dir(&temp.0).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
    }
    #[test]
    fn invalid_utf8_terminal_contents_abort_without_overwrite() {
        let temp = Temp::new();
        let path = temp.0.join("xdg-terminals.list");
        fs::write(&path, b"old.desktop\n\xff").unwrap();
        assert_eq!(
            set_terminal(&path, "chosen.desktop").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(fs::read(&path).unwrap(), b"old.desktop\n\xff");
    }
    #[test]
    fn configuration_lock_serializes_concurrent_read_modify_writes() {
        let temp = Temp::new();
        let config = Arc::new(temp.0.clone());
        let list = config.join("xdg-terminals.list");
        fs::write(&list, "# retained\noriginal.desktop\n").unwrap();
        let threads: Vec<_> = (0..12)
            .map(|index| {
                let config = config.clone();
                std::thread::spawn(move || {
                    let _lock = ConfigLock::acquire(&config).unwrap();
                    set_terminal(
                        &config.join("xdg-terminals.list"),
                        &format!("choice-{index}.desktop"),
                    )
                    .unwrap();
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let text = fs::read_to_string(list).unwrap();
        let lines: HashSet<_> = text.lines().collect();
        assert_eq!(lines.len(), 14);
        assert!(lines.contains("# retained"));
        assert!(lines.contains("original.desktop"));
        for index in 0..12 {
            assert!(lines.contains(format!("choice-{index}.desktop").as_str()));
        }
        assert_eq!(
            config
                .join(".apps-defaults.lock")
                .metadata()
                .unwrap()
                .mode()
                & 0o777,
            0o600
        );
    }
    #[test]
    fn configuration_lock_never_follows_a_symlink() {
        let temp = Temp::new();
        let sentinel = temp.0.join("sentinel");
        fs::write(&sentinel, b"unchanged").unwrap();
        symlink("sentinel", temp.0.join(".apps-defaults.lock")).unwrap();
        assert!(ConfigLock::acquire(&temp.0).is_err());
        assert_eq!(fs::read(sentinel).unwrap(), b"unchanged");
    }
    #[test]
    fn desktop_specific_override_list_preserves_desktop_order_and_existing_files() {
        let temp = Temp::new();
        fs::write(temp.0.join("gnome-mimeapps.list"), b"# existing").unwrap();
        fs::write(temp.0.join("unity-mimeapps.list"), b"# existing").unwrap();
        assert_eq!(
            desktop_mimeapps(&temp.0, "GNOME:missing:Unity:GNOME"),
            [
                temp.0.join("gnome-mimeapps.list"),
                temp.0.join("unity-mimeapps.list"),
                temp.0.join("gnome-mimeapps.list")
            ]
        );
    }
}
