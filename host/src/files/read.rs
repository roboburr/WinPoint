//! Native local-folder metadata, preserving the existing Explorer wire format.
//! Launcher names, icons and trust rules use the same GIO/key-file contract as the reference.
use super::{ApiError, ApiResult, Context, abs_in, io_error};
use gio::prelude::*;
use glib::Unichar;
use serde_json::{Value, json};
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    fs::{self, OpenOptions},
    io::Read,
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

const MAX_LIST: usize = 50_000;

pub(super) fn call(method: &str, args: &Value, ctx: &Context) -> Option<ApiResult> {
    match method {
        "files.list" => {
            let raw = &args["path"];
            if raw.as_str().is_some_and(|s| s.starts_with("trash:")) {
                return None;
            }
            let path = if truthy(raw) {
                match abs_in(raw, ctx.lang()) {
                    Ok(p) => p,
                    Err(e) => return Some(Err(e)),
                }
            } else {
                ctx.home.clone()
            };
            let show_hidden = args
                .get("showHidden")
                .filter(|v| !v.is_null())
                .map(truthy)
                .unwrap_or_else(|| ctx.show_hidden());
            match folder(&path, show_hidden, MAX_LIST, ctx) {
                Ok(Some((items, truncated))) => {
                    Some(Ok(json!({"path":path,"parent":path.parent(),"items":items,
                    "truncated":truncated,"writable":writable(&path)})))
                }
                Ok(None) => None,
                Err(e) => Some(Err(e)),
            }
        }
        "files.stat" => {
            let path = match abs_in(&args["path"], ctx.lang()) {
                Ok(p) => p,
                Err(e) => return Some(Err(e)),
            };
            Some(item(&path, ctx))
        }
        "files.desktop" => {
            let path = &ctx.desktop;
            if !path.is_dir() {
                return Some(Ok(json!({"dir":path,"items":[],"exists":false})));
            }
            match folder(path, truthy(&args["showHidden"]), 5000, ctx) {
                Ok(Some((items, truncated))) => Some(Ok(
                    json!({"dir":path,"items":items,"exists":true,"truncated":truncated}),
                )),
                Ok(None) => None,
                Err(e) => Some(Err(e)),
            }
        }
        _ => None,
    }
}

pub(super) fn item(path: &Path, ctx: &Context) -> Result<Value, ApiError> {
    let hidden = hidden_names(path.parent().unwrap_or(Path::new("/")));
    Factory::new(ctx).make(path, &hidden)
}

pub(super) fn filename_mime(name: &str) -> String {
    let Ok(filename) = std::ffi::CString::new(name) else {
        return "application/octet-stream".into();
    };
    let mut uncertain = glib::ffi::GFALSE;
    let content_type: glib::GString = unsafe {
        glib::translate::from_glib_full(gio::ffi::g_content_type_guess(
            filename.as_ptr(),
            std::ptr::null(),
            0,
            &mut uncertain,
        ))
    };
    gio::content_type_get_mime_type(&content_type)
        .map(|value| value.to_string())
        .unwrap_or_else(|| {
            if content_type.is_empty() {
                "application/octet-stream".into()
            } else {
                content_type.to_string()
            }
        })
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(v) => *v,
        Value::Number(v) => v.as_f64().is_some_and(|n| n != 0.0),
        Value::String(v) => !v.is_empty(),
        Value::Array(v) => !v.is_empty(),
        Value::Object(v) => !v.is_empty(),
    }
}

fn writable(path: &Path) -> bool {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    CString::new(path.as_os_str().as_bytes())
        .is_ok_and(|path| unsafe { libc::access(path.as_ptr(), libc::W_OK) == 0 })
}

type Listing = Option<(Vec<Value>, bool)>;
fn folder(
    path: &Path,
    show_hidden: bool,
    limit: usize,
    ctx: &Context,
) -> Result<Listing, ApiError> {
    let lang = ctx.lang();
    let meta = fs::metadata(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ApiError::tr(
                "not_found",
                lang,
                "files.folderGone",
                crate::i18n::NO_PARAMS,
            )
        } else {
            io_error(e, path, lang)
        }
    })?;
    if !meta.is_dir() {
        return Err(ApiError::tr(
            "not_dir",
            lang,
            "files.notDir",
            crate::i18n::NO_PARAMS,
        ));
    }
    let entries = fs::read_dir(path).map_err(|e| io_error(e, path, lang))?;
    let hidden = hidden_names(path);
    let mut factory = Factory::new(ctx);
    let mut items = Vec::new();
    let mut truncated = false;
    for entry in entries {
        let entry = entry.map_err(|e| io_error(e, path, lang))?;
        if entry.file_name().to_str().is_none() {
            continue;
        }
        if items.len() >= limit {
            truncated = true;
            break;
        }
        let Ok(item) = factory.make(&entry.path(), &hidden) else {
            continue;
        };
        if !show_hidden && item["hidden"] == true {
            continue;
        }
        items.push(item);
    }
    // Normalize names once per item, like Python's key= sort, rather than for
    // every comparison in a directory containing tens of thousands of entries.
    let mut keyed: Vec<_> = items
        .into_iter()
        .map(|item| {
            let is_dir = item["isDir"].as_bool().unwrap_or(false);
            let name = natural_key(item["displayName"].as_str().unwrap_or(""));
            (is_dir, name, item)
        })
        .collect();
    keyed.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| compare_natural_keys(&a.1, &b.1)));
    let items = keyed.into_iter().map(|(_, _, item)| item).collect();
    Ok(Some((items, truncated)))
}

pub(super) fn hidden_names(path: &Path) -> HashSet<String> {
    let path = path.join(".hidden");
    if !path.is_file() {
        return HashSet::new();
    }
    let Ok(file) = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
    else {
        return HashSet::new();
    };
    let mut bytes = Vec::new();
    if file.take(1024 * 1024).read_to_end(&mut bytes).is_err() {
        return HashSet::new();
    }
    String::from_utf8_lossy(&bytes)
        .chars()
        .take(256 * 1024)
        .collect::<String>()
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(super) struct Factory<'a> {
    ctx: &'a Context,
    sniff: usize,
    icons: Arc<HashSet<String>>,
}
impl<'a> Factory<'a> {
    pub(super) fn new(ctx: &'a Context) -> Self {
        Self {
            ctx,
            sniff: 300,
            icons: icon_catalog(),
        }
    }

    pub(super) fn make(
        &mut self,
        path: &Path,
        hidden_names: &HashSet<String>,
    ) -> Result<Value, ApiError> {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("/");
        let lst = fs::symlink_metadata(path).map_err(|e| io_error(e, path, self.ctx.lang()))?;
        let is_link = lst.file_type().is_symlink();
        let (st, broken) = if is_link {
            match fs::metadata(path) {
                Ok(st) => (st, false),
                Err(_) => (lst, true),
            }
        } else {
            (lst, false)
        };
        let is_dir = st.is_dir() && !broken;
        let launcher = st.is_file() && name.ends_with(".desktop");
        let mut display = name.to_owned();
        let mut is_desktop = false;
        let mut shell_shortcut = None;
        let mut untrusted = false;
        let (mime, icon) = if is_dir {
            ("inode/directory".to_owned(), self.folder_icon(path))
        } else if broken {
            (
                "inode/symlink".to_owned(),
                self.first_icon(&["emblem-symbolic-link"], "text-x-generic"),
            )
        } else if st.is_file() {
            if launcher {
                let icon = if let Some(entry) = desktop_entry(path, &self.ctx.language) {
                    is_desktop = true;
                    shell_shortcut = entry.shell_shortcut;
                    if !entry.name.is_empty() {
                        display = entry.name;
                    }
                    if entry.kind == "Application" && !launcher_trusted(path, &st) {
                        display = name.to_owned();
                        untrusted = true;
                        self.first_icon(
                            &["application-x-desktop", "application-x-executable"],
                            "text-x-generic",
                        )
                    } else if entry.icon.is_empty() {
                        "application-x-executable".into()
                    } else {
                        entry.icon
                    }
                } else {
                    self.mime_icon("application/x-desktop")
                };
                ("application/x-desktop".to_owned(), icon)
            } else {
                let mime = self.guess_mime(path, name, st.len());
                let icon = self.mime_icon(&mime);
                (mime, icon)
            }
        } else {
            let kind = st.file_type();
            let mime = if kind.is_fifo() {
                "inode/fifo"
            } else if kind.is_socket() {
                "inode/socket"
            } else if kind.is_block_device() {
                "inode/blockdevice"
            } else {
                "inode/chardevice"
            };
            (mime.to_owned(), "application-x-generic".to_owned())
        };
        let modified = (i128::from(st.mtime()) * 1000 + i128::from(st.mtime_nsec()) / 1_000_000)
            .clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64;
        let mut result = json!({"name":name,"displayName":display,"path":path,
            "uri":gio::File::for_path(path).uri().to_string(),"isDir":is_dir,"isDesktopEntry":is_desktop,
            "icon":icon,"mime":mime,"size":if is_dir {0} else {st.len()},"modified":modified,
            "hidden":name.starts_with('.') || name.ends_with('~') || hidden_names.contains(name),
            "isSymlink":is_link,"typeName":type_name(&mime,name,&self.ctx.language)});
        if let Some(shortcut) = shell_shortcut {
            result["shellShortcut"] = json!(shortcut);
        }
        if untrusted {
            result["untrusted"] = json!(true);
        }
        Ok(result)
    }

    pub(super) fn first_icon(&self, names: &[&str], fallback: &str) -> String {
        names
            .iter()
            .find(|name| self.icons.contains(**name))
            .copied()
            .unwrap_or(fallback)
            .to_owned()
    }

    fn folder_icon(&self, path: &Path) -> String {
        if path == self.ctx.home {
            return self.first_icon(&["user-home"], "folder");
        }
        if path == self.ctx.desktop {
            return self.first_icon(&["user-desktop"], "folder");
        }
        for (kind, defaults, names) in [
            (
                glib::UserDirectory::Documents,
                "Documents",
                &["folder-documents"][..],
            ),
            (
                glib::UserDirectory::Downloads,
                "Downloads",
                &["folder-download", "folder-downloads"][..],
            ),
            (glib::UserDirectory::Music, "Music", &["folder-music"][..]),
            (
                glib::UserDirectory::Pictures,
                "Pictures",
                &["folder-pictures"][..],
            ),
            (
                glib::UserDirectory::Videos,
                "Videos",
                &["folder-videos"][..],
            ),
            (
                glib::UserDirectory::Templates,
                "Templates",
                &["folder-templates"][..],
            ),
            (
                glib::UserDirectory::PublicShare,
                "Public",
                &["folder-publicshare"][..],
            ),
        ] {
            let special =
                glib::user_special_dir(kind).unwrap_or_else(|| self.ctx.home.join(defaults));
            if path == special {
                return self.first_icon(names, "folder");
            }
        }
        "folder".to_owned()
    }

    fn guess_mime(&mut self, path: &Path, name: &str, size: u64) -> String {
        // Rust's safe wrapper passes a non-null empty slice, which GLib treats as
        // evidence of an empty file. The reference first performs a filename-only
        // guess with data=NULL; only an uncertain result is sniffed afterwards.
        let filename = std::ffi::CString::new(name).expect("filesystem names cannot contain NUL");
        let mut uncertain = glib::ffi::GFALSE;
        let mut content_type: glib::GString = unsafe {
            glib::translate::from_glib_full(gio::ffi::g_content_type_guess(
                filename.as_ptr(),
                std::ptr::null(),
                0,
                &mut uncertain,
            ))
        };
        let uncertain = uncertain != glib::ffi::GFALSE;
        if uncertain && size > 0 && self.sniff > 0 {
            self.sniff -= 1;
            if let Ok(file) = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
                .open(path)
            {
                let mut head = Vec::new();
                if file.take(4096).read_to_end(&mut head).is_ok() {
                    let (sniffed, _) = gio::content_type_guess(Some(Path::new(name)), &head);
                    if !sniffed.is_empty() {
                        content_type = sniffed;
                    }
                }
            }
        }
        gio::content_type_get_mime_type(&content_type)
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                if content_type.is_empty() {
                    "application/octet-stream".to_owned()
                } else {
                    content_type.to_string()
                }
            })
    }

    fn mime_icon(&self, mime: &str) -> String {
        let mut candidates = Vec::new();
        if let Ok(themed) = gio::content_type_get_icon(mime).downcast::<gio::ThemedIcon>() {
            candidates.extend(themed.names().iter().map(|name| name.to_string()));
        }
        if let Some(generic) = gio::content_type_get_generic_icon_name(mime) {
            candidates.push(generic.to_string());
        }
        let major = mime.split('/').next().unwrap_or("application");
        candidates.extend([mime.replace('/', "-"), format!("{major}-x-generic")]);
        if mime == "inode/directory" {
            candidates.insert(0, "folder".to_owned());
        }
        let fallback = if major == "text" {
            "text-x-generic"
        } else {
            "application-x-generic"
        };
        candidates.extend([fallback.to_owned(), "unknown".to_owned()]);
        candidates.sort_by_key(|name| name.ends_with("-symbolic"));
        candidates
            .into_iter()
            .find(|name| self.icons.contains(name))
            .unwrap_or_else(|| fallback.to_owned())
    }
}

struct DesktopEntry {
    kind: String,
    name: String,
    icon: String,
    shell_shortcut: Option<&'static str>,
}
fn own_computer_shortcut(path: &Path, exec: &str, expected: &Path) -> bool {
    // This is the explicit legacy shortcut created for this port. Never infer a
    // special item from a label/icon alone or modify the actual desktop file.
    if !matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("Deze PC.desktop" | "This PC.desktop")
    ) {
        return false;
    }
    glib::shell_parse_argv(exec).is_ok_and(|argv| {
        argv.len() == 2 && argv[0].as_os_str() == expected.as_os_str() && argv[1] == "computer:///"
    })
}
fn desktop_entry(path: &Path, language: &str) -> Option<DesktopEntry> {
    let keyfile = glib::KeyFile::new();
    keyfile
        .load_from_file(path, glib::KeyFileFlags::KEEP_TRANSLATIONS)
        .ok()?;
    if !keyfile.has_group("Desktop Entry") {
        return None;
    }
    let text = |key: &str| {
        keyfile
            .string("Desktop Entry", key)
            .map(|value| value.to_string())
            .unwrap_or_default()
    };
    let base = text("Name");
    let mut name = keyfile
        .locale_string("Desktop Entry", "Name", Some(language))
        .map(|value| value.to_string())
        .unwrap_or_default();
    if name.is_empty() {
        name = base.clone();
    }
    static NAMES: OnceLock<Value> = OnceLock::new();
    let names = NAMES.get_or_init(|| {
        serde_json::from_str(include_str!("launcher-names.json")).expect("bundled launcher names")
    });
    let basename = path.file_name()?.to_str()?;
    if language == "nl"
        && name == base
        && let Some(translated) = names["NL_NAMES"][basename].as_str()
    {
        name = translated.into();
    }
    if let Some(translated) = names["SAME_NAME_AS_OURS"][basename][language].as_str() {
        name = translated.into();
    }
    let shell_shortcut = std::env::current_exe()
        .ok()
        .map(|exe| exe.with_file_name("win11-explorer"))
        .filter(|exe| own_computer_shortcut(path, &text("Exec"), exe))
        .filter(|_| matches!(text("Type").as_str(), "" | "Application"))
        .map(|_| "thispc");
    if shell_shortcut.is_some() {
        name = crate::i18n::tr(language, "files.thisPc");
    }
    let mut icon = text("Icon");
    if !icon.starts_with('/') {
        for suffix in [".png", ".svg", ".xpm"] {
            if icon.to_lowercase().ends_with(suffix) {
                icon.truncate(icon.len() - suffix.len());
                break;
            }
        }
    }
    let kind = text("Type");
    Some(DesktopEntry {
        kind: if kind.is_empty() {
            "Application".into()
        } else {
            kind
        },
        name,
        icon,
        shell_shortcut,
    })
}

fn launcher_trusted(path: &Path, metadata: &fs::Metadata) -> bool {
    let real = fs::canonicalize(path).unwrap_or_else(|_| path.into());
    let mut dirs = vec![
        glib::user_data_dir().join("applications"),
        PathBuf::from("/var/lib/flatpak/exports/share/applications"),
        PathBuf::from("/var/lib/snapd/desktop/applications"),
        glib::user_data_dir().join("flatpak/exports/share/applications"),
    ];
    dirs.extend(
        glib::system_data_dirs()
            .iter()
            .map(|path| path.join("applications")),
    );
    if dirs
        .into_iter()
        .any(|dir| real.starts_with(fs::canonicalize(&dir).unwrap_or(dir)))
    {
        return true;
    }
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(gio::DesktopAppInfo::new)
        .is_some()
    {
        return true;
    }
    if metadata.uid() != unsafe { libc::getuid() } || metadata.mode() & 0o100 == 0 {
        return false;
    }
    let no_permissions = [
        "vfat",
        "msdos",
        "exfat",
        "ntfs",
        "ntfs3",
        "fuseblk",
        "fuse.ntfs-3g",
        "fuse.exfat",
        "udf",
        "iso9660",
        "hfsplus",
        "fuse.sshfs",
        "cifs",
        "smb3",
        "9p",
    ];
    if no_permissions.contains(&filesystem_type(&real).as_str()) {
        return false;
    }
    gio::File::for_path(path)
        .query_info(
            "metadata::trusted",
            gio::FileQueryInfoFlags::NONE,
            gio::Cancellable::NONE,
        )
        .ok()
        .and_then(|info| info.attribute_as_string("metadata::trusted"))
        .is_some_and(|value| matches!(value.trim().to_lowercase().as_str(), "true" | "yes" | "1"))
}

pub(super) fn filesystem_type(path: &Path) -> String {
    let Ok(mounts) = fs::read_to_string("/proc/self/mountinfo") else {
        return String::new();
    };
    let mut best = 0;
    let mut kind = String::new();
    for line in mounts.lines() {
        let parts: Vec<_> = line.split_whitespace().collect();
        if parts.len() < 10 {
            continue;
        }
        let Some(separator) = parts.iter().position(|part| *part == "-") else {
            continue;
        };
        let mount = parts[4]
            .replace("\\040", " ")
            .replace("\\011", "\t")
            .replace("\\134", "\\");
        if mount.len() >= best && path.starts_with(&mount) {
            best = mount.len();
            kind = parts.get(separator + 1).unwrap_or(&"").to_string();
        }
    }
    kind
}

pub(super) fn type_name(mime: &str, name: &str, lang: &str) -> String {
    static TYPES: OnceLock<Value> = OnceLock::new();
    let types = TYPES.get_or_init(|| {
        serde_json::from_str(include_str!("types.json")).expect("bundled MIME names")
    });
    let nl = lang == "nl";
    if let Some(value) = types[mime][if nl { 0 } else { 1 }].as_str() {
        return value.to_owned();
    }
    let base = name.trim_start_matches('.');
    let ext = base.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");
    let major = mime.split('/').next().unwrap_or("");
    let file_type = || {
        crate::i18n::trf(
            lang,
            "files.type.ext",
            &serde_json::json!({"ext": ext.to_uppercase()}),
        )
    };
    if !ext.is_empty() && ["image", "audio", "video"].contains(&major) && ext.chars().count() <= 6 {
        return file_type();
    }
    if major == "font" {
        return crate::i18n::tr(lang, "files.type.font");
    }
    if mime != "application/octet-stream" {
        if let Some(comment) = mime_comment(mime, lang) {
            return comment;
        }
    }
    if !ext.is_empty() && ext.chars().count() <= 8 {
        return file_type();
    }
    crate::i18n::tr(lang, "files.type.file")
}

fn mime_comment(mime: &str, lang: &str) -> Option<String> {
    static COMMENTS: OnceLock<Mutex<HashMap<(String, String), Option<String>>>> = OnceLock::new();
    let key = (mime.to_owned(), lang.to_owned());
    let mut cache = COMMENTS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(value) = cache.get(&key) {
        return value.clone();
    }
    let valid = mime.split_once('/').is_some_and(|(major, minor)| {
        !major.is_empty()
            && !minor.is_empty()
            && major
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b".+-".contains(&b))
            && minor
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".+_-".contains(&b))
    });
    let mut result = None;
    if valid {
        for directory in data_dirs() {
            let Ok(text) = fs::read_to_string(directory.join("mime").join(format!("{mime}.xml")))
            else {
                continue;
            };
            let mut default = None;
            let mut rest = text.as_str();
            while let Some(index) = rest.find("<comment") {
                rest = &rest[index + 8..];
                let Some((attributes, body)) = rest.split_once('>') else {
                    break;
                };
                let Some((value, tail)) = body.split_once("</comment>") else {
                    break;
                };
                rest = tail;
                let value = value.trim();
                let locale = attributes
                    .split_once("xml:lang=\"")
                    .and_then(|(_, v)| v.split('"').next());
                if locale.is_none() && default.is_none() {
                    default = Some(value.to_owned());
                } else if locale.is_some_and(|locale| {
                    [lang, &format!("{lang}_NL"), &format!("{lang}_BE")].contains(&locale)
                }) && !value.is_empty()
                {
                    result = Some(value.to_owned());
                    break;
                }
            }
            if result.is_none() && lang == "en" {
                result = default;
            }
            break;
        }
    }
    result = result.filter(|s| !s.is_empty()).map(|s| {
        let s = s
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">");
        let mut chars = s.chars();
        chars
            .next()
            .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
            .unwrap_or_default()
    });
    cache.insert(key, result.clone());
    result
}

#[derive(Debug)]
struct IconCache {
    theme: String,
    at: Instant,
    names: Arc<HashSet<String>>,
}

pub(super) fn icon_catalog() -> Arc<HashSet<String>> {
    static CACHE: OnceLock<Mutex<Option<IconCache>>> = OnceLock::new();
    let theme = current_theme();
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(value) = cache
        .as_ref()
        .filter(|c| c.theme == theme && c.at.elapsed() < Duration::from_secs(30))
    {
        return value.names.clone();
    }
    let mut bases: Vec<PathBuf> = data_dirs().into_iter().map(|p| p.join("icons")).collect();
    bases.push(glib::home_dir().join(".icons"));
    bases.extend([
        PathBuf::from("/usr/share/icons"),
        PathBuf::from("/var/lib/flatpak/exports/share/icons"),
        glib::user_data_dir().join("flatpak/exports/share/icons"),
        PathBuf::from("/var/lib/snapd/desktop/icons"),
    ]);
    let names = Arc::new(build_icon_catalog(&theme, &bases));
    *cache = Some(IconCache {
        theme,
        at: Instant::now(),
        names: names.clone(),
    });
    names
}

/// How long [`current_theme`] keeps the icon theme it read (R2-D m1).
const THEME_TTL: Duration = Duration::from_secs(2);
#[cfg(test)]
static THEME_READS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The icon theme, read at most once per [`THEME_TTL`] per process, under one lock (R2-D m1, GX1 n1): it used to
/// build a GSettings on every [`icon_catalog`] call, before the catalogue's own cache was checked, in every
/// thread at once; a theme change still shows within 2 s.
fn current_theme() -> String {
    if let Ok(theme) = std::env::var("WIN11SHELL_ICON_THEME") {
        if !theme.is_empty() {
            return theme;
        }
    }
    static THEME: Mutex<Option<(Instant, String)>> = Mutex::new(None);
    let mut cached = THEME.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, theme)) = cached.as_ref()
        && at.elapsed() < THEME_TTL
    {
        return theme.clone();
    }
    let theme = read_theme();
    *cached = Some((Instant::now(), theme.clone()));
    theme
}

/// The test binary only (the intermittent unit-suite hang: R2-D m1, GX1 n1, GW4 §3.1). The GTK hosts set up GIO's
/// process-wide defaults on their main thread (gtk::init) before any worker thread ran (GTK left the host on 14-11;
/// a `--http-only` backend never initialised it); the lib test binary did that inside up to 16 tests at the same
/// moment (app_params and files::actions through g_vfs_get_default,
/// apps::catalog, files::places and files::properties through the icon theme's GSettings and its default
/// backend, apps::recent through the MIME database), and every recorded hang had exactly those tests blocked
/// in it. So the binary does it once, on one thread, before libtest starts its test threads: the default VFS
/// (local in the sandbox, GIO_USE_VFS), the GSettings schema source and backend, the MIME database. Only in the
/// sandboxed suite (tests/rust-sandbox/run-tests.sh sets GIO_USE_VFS=local and GSETTINGS_BACKEND=keyfile): any
/// other run of the binary (`--list`, a plain `cargo test`, the update bus tests) never builds GVfs's daemon VFS
/// or a dconf GSettings before `main` (R3-NATIVE review m5).
#[cfg(test)]
#[used]
#[unsafe(link_section = ".init_array")]
static GIO_DEFAULTS_BEFORE_THE_TEST_THREADS: extern "C" fn() = {
    extern "C" fn set_up() {
        if std::env::var("GIO_USE_VFS").as_deref() != Ok("local")
            || std::env::var("GSETTINGS_BACKEND").as_deref() != Ok("keyfile")
        {
            return;
        }
        let _ = gio::Vfs::default();
        let _ = current_theme();
        let _ = gio::content_type_guess(Some("set-up.txt"), &[]);
    }
    set_up
};

fn read_theme() -> String {
    #[cfg(test)]
    THEME_READS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    if let Some(schema) = gio::SettingsSchemaSource::default()
        .and_then(|s| s.lookup("org.gnome.desktop.interface", true))
    {
        let settings = gio::Settings::new_full(&schema, None::<&gio::SettingsBackend>, None);
        let theme = settings.string("icon-theme");
        if !theme.is_empty() {
            return theme.to_string();
        }
    }
    let file = glib::KeyFile::new();
    if file
        .load_from_file(
            glib::user_config_dir().join("gtk-3.0/settings.ini"),
            glib::KeyFileFlags::NONE,
        )
        .is_ok()
    {
        if let Ok(theme) = file.string("Settings", "gtk-icon-theme-name") {
            if !theme.is_empty() {
                return theme.to_string();
            }
        }
    }
    "hicolor".to_owned()
}

fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![glib::user_data_dir()];
    dirs.extend(
        std::env::var_os("XDG_DATA_DIRS")
            .filter(|s| !s.is_empty())
            .map(|s| std::env::split_paths(&s).collect::<Vec<_>>())
            .unwrap_or_else(|| {
                vec![
                    PathBuf::from("/usr/local/share"),
                    PathBuf::from("/usr/share"),
                ]
            })
            .into_iter()
            .filter(|p| p.is_absolute()),
    );
    dirs
}

fn build_icon_catalog(theme: &str, bases: &[PathBuf]) -> HashSet<String> {
    fn add(
        theme: &str,
        bases: &[PathBuf],
        seen: &mut HashSet<String>,
        names: &mut HashSet<String>,
        depth: usize,
    ) {
        if depth > 10 || !seen.insert(theme.to_owned()) || theme.contains('/') || theme == ".." {
            return;
        }
        let Some(index) = bases
            .iter()
            .map(|base| base.join(theme).join("index.theme"))
            .find(|p| p.is_file())
        else {
            return;
        };
        let file = glib::KeyFile::new();
        if file
            .load_from_file(index, glib::KeyFileFlags::NONE)
            .is_err()
        {
            return;
        }
        let directories = format!(
            "{},{}",
            file.string("Icon Theme", "Directories").unwrap_or_default(),
            file.string("Icon Theme", "ScaledDirectories")
                .unwrap_or_default()
        );
        for directory in directories
            .split(',')
            .map(str::trim)
            .filter(|d| !d.is_empty())
        {
            if Path::new(directory).is_absolute()
                || !file.has_group(directory)
                || Path::new(directory)
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                continue;
            }
            for base in bases {
                collect_icon_names(&base.join(theme).join(directory), names);
            }
        }
        for parent in file
            .string("Icon Theme", "Inherits")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|n| *n != "hicolor" && !n.is_empty())
        {
            add(parent, bases, seen, names, depth + 1);
        }
    }
    let mut seen = HashSet::new();
    let mut names = HashSet::new();
    add(theme, bases, &mut seen, &mut names, 0);
    add("Adwaita", bases, &mut seen, &mut names, 0);
    add("hicolor", bases, &mut seen, &mut names, 0);
    for base in bases
        .iter()
        .chain(std::iter::once(&PathBuf::from("/usr/share/pixmaps")))
    {
        collect_icon_names(base, &mut names);
    }
    names
}

fn collect_icon_names(directory: &Path, names: &mut HashSet<String>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some((stem, ext)) = name.rsplit_once('.') else {
            continue;
        };
        if !stem.is_empty() && ["svg", "png", "xpm"].contains(&ext.to_ascii_lowercase().as_str()) {
            names.insert(stem.to_owned());
        }
    }
}

fn natural_key(text: &str) -> Vec<(bool, String)> {
    let normalized = glib::normalize(text, glib::NormalizeMode::All);
    let folded = glib::casefold(normalized);
    let mut out: Vec<(bool, String)> = Vec::new();
    for ch in folded.chars().filter(|c| c.combining_class() == 0) {
        let digit = unsafe { glib::ffi::g_unichar_digit_value(ch as u32) };
        let numeric = digit >= 0;
        let text = if numeric {
            char::from(b'0' + digit as u8)
        } else {
            ch
        };
        if out.last().is_some_and(|(kind, _)| *kind == numeric) {
            out.last_mut().unwrap().1.push(text);
        } else {
            out.push((numeric, text.to_string()));
        }
    }
    out
}

#[cfg(test)]
fn natural_cmp(a: &str, b: &str) -> Ordering {
    compare_natural_keys(&natural_key(a), &natural_key(b))
}

fn compare_natural_keys(a: &[(bool, String)], b: &[(bool, String)]) -> Ordering {
    for ((an, a), (bn, b)) in a.iter().zip(b) {
        let order = bn.cmp(an).then_with(|| {
            if *an {
                let a = a.trim_start_matches('0');
                let b = b.trim_start_matches('0');
                a.len().cmp(&b.len()).then_with(|| a.cmp(b))
            } else {
                a.cmp(b)
            }
        });
        if order != Ordering::Equal {
            return order;
        }
    }
    a.len().cmp(&b.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    #[test]
    fn the_icon_theme_is_read_once_for_many_threads_and_the_catalogue_is_shared() {
        // R2-D m1: 16 threads at once (the cold start of the unit suite) read the theme at most once, not 16
        // GSettings; within THEME_TTL there is at most one expiry, so the reads grow by 0 or 1
        let before = THEME_READS.load(std::sync::atomic::Ordering::SeqCst);
        let start = std::sync::Arc::new(std::sync::Barrier::new(16));
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let start = start.clone();
                std::thread::spawn(move || {
                    start.wait();
                    (current_theme(), icon_catalog())
                })
            })
            .collect();
        let threads: Vec<_> = handles
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        let reads = THEME_READS.load(std::sync::atomic::Ordering::SeqCst) - before;
        assert!(reads <= 1, "{reads} theme reads for 16 threads");
        assert!(threads.iter().all(|(theme, _)| *theme == threads[0].0));
        assert!(threads.iter().all(|(_, names)| **names == *threads[0].1));
    }
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "win11-read-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
        fn context(&self) -> Context {
            Context::new(self.0.clone(), self.0.join("Desktop"), "en".into(), false)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn natural_order_matches_numbers_accents_and_casefold() {
        assert_eq!(natural_cmp("File 2", "File 10"), Ordering::Less);
        assert_eq!(natural_cmp("École", "ecole"), Ordering::Equal);
        assert_eq!(natural_cmp("Straße", "STRASSE"), Ordering::Equal);
        assert_eq!(natural_cmp("file 0002", "file 2"), Ordering::Equal);
        assert_eq!(
            natural_cmp(
                "file 999999999999999999999999",
                "file 1000000000000000000000000"
            ),
            Ordering::Less
        );
    }
    #[test]
    fn listing_preserves_hidden_link_and_broken_link_metadata() {
        let f = Fixture::new();
        let ctx = f.context();
        fs::create_dir(f.0.join("Folder 10")).unwrap();
        fs::create_dir(f.0.join("Folder 2")).unwrap();
        fs::write(f.0.join("a file %.txt"), b"hello").unwrap();
        fs::write(f.0.join(".secret"), b"hidden").unwrap();
        fs::write(f.0.join("backup~"), b"hidden").unwrap();
        fs::write(f.0.join("custom"), b"hidden").unwrap();
        fs::write(f.0.join(".hidden"), b" custom \n").unwrap();
        symlink("Folder 2", f.0.join("linked")).unwrap();
        symlink("absent", f.0.join("broken")).unwrap();
        let result = call("files.list", &json!({"path":f.0}), &ctx)
            .unwrap()
            .unwrap();
        let rows = result["items"].as_array().unwrap();
        assert_eq!(
            rows.iter()
                .take(3)
                .map(|r| r["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["Folder 2", "Folder 10", "linked"]
        );
        assert!(!rows.iter().any(|r| {
            [".secret", "backup~", "custom", ".hidden"].contains(&r["name"].as_str().unwrap())
        }));
        let link = rows.iter().find(|r| r["name"] == "linked").unwrap();
        assert_eq!(link["isSymlink"], true);
        assert_eq!(link["isDir"], true);
        assert_eq!(link["size"], 0);
        let broken = rows.iter().find(|r| r["name"] == "broken").unwrap();
        assert_eq!(broken["mime"], "inode/symlink");
        assert_eq!(broken["typeName"], "Shortcut (broken)");
        let file = rows.iter().find(|r| r["name"] == "a file %.txt").unwrap();
        assert!(file["uri"].as_str().unwrap().contains("a%20file%20%25.txt"));
        assert_eq!(file["size"], 5);
        assert_eq!(item(&f.0.join("custom"), &ctx).unwrap()["hidden"], true);
    }
    #[test]
    fn mime_guess_distinguishes_filename_only_from_empty_content() {
        let f = Fixture::new();
        let ctx = f.context();
        fs::write(f.0.join("empty.txt"), b"").unwrap();
        fs::write(f.0.join("empty.json"), b"").unwrap();
        fs::write(f.0.join("één.json"), b"{\"value\":1}\n").unwrap();
        fs::write(f.0.join("source"), b"some plain text\n").unwrap();
        symlink("source", f.0.join("link-file")).unwrap();
        assert_eq!(
            item(&f.0.join("empty.txt"), &ctx).unwrap()["mime"],
            "text/plain"
        );
        assert_eq!(
            item(&f.0.join("empty.json"), &ctx).unwrap()["mime"],
            "application/json"
        );
        assert_eq!(
            item(&f.0.join("één.json"), &ctx).unwrap()["mime"],
            "application/json"
        );
        assert_eq!(
            item(&f.0.join("link-file"), &ctx).unwrap()["mime"],
            "text/plain"
        );
    }
    #[test]
    fn launchers_have_native_metadata_and_trash_is_dispatched_separately() {
        let f = Fixture::new();
        let ctx = f.context();
        fs::write(
            f.0.join("fake.desktop"),
            b"[Desktop Entry]\nType=Application\nName=photo.jpg\nExec=false\nIcon=image-jpeg\n",
        )
        .unwrap();
        assert!(
            call("files.list", &json!({"path":f.0}), &ctx)
                .unwrap()
                .is_ok()
        );
        assert!(
            call(
                "files.stat",
                &json!({"path":f.0.join("fake.desktop")}),
                &ctx
            )
            .unwrap()
            .is_ok()
        );
        assert!(call("files.list", &json!({"path":"trash:///"}), &ctx).is_none());
        let metadata = item(&f.0.join("fake.desktop"), &ctx).unwrap();
        assert_eq!(metadata["displayName"], "fake.desktop");
        assert_eq!(metadata["isDesktopEntry"], true);
        assert_eq!(metadata["untrusted"], true);
    }
    #[test]
    fn desktop_launcher_names_follow_requested_shell_language() {
        let f = Fixture::new();
        let ordinary = f.0.join("localized.desktop");
        fs::write(&ordinary, "[Desktop Entry]\nType=Application\nName=Fallback\nName[nl]=Nederlandse naam\nName[en]=English name\nExec=/usr/bin/true\n").unwrap();
        assert_eq!(
            desktop_entry(&ordinary, "nl").unwrap().name,
            "Nederlandse naam"
        );
        assert_eq!(desktop_entry(&ordinary, "en").unwrap().name, "English name");
        let legacy = f.0.join("Deze PC.desktop");
        let expected = std::env::current_exe()
            .unwrap()
            .with_file_name("win11-explorer");
        let content = format!(
            "[Desktop Entry]\nType=Application\nName=Deze PC\nExec={} computer:///\n",
            glib::shell_quote(expected.to_str().unwrap()).to_string_lossy()
        );
        fs::write(&legacy, &content).unwrap();
        let dutch = desktop_entry(&legacy, "nl").unwrap();
        let english = desktop_entry(&legacy, "en").unwrap();
        assert_eq!(dutch.name, "Deze pc");
        assert_eq!(english.name, "This PC");
        assert_eq!(dutch.shell_shortcut, Some("thispc"));
        assert_eq!(fs::read_to_string(&legacy).unwrap(), content);
    }
    #[test]
    fn legacy_computer_shortcut_requires_exact_own_command_and_known_filename() {
        let expected = Path::new("/tmp/prefix with spaces/bin/win11-explorer");
        let exec = format!(
            "{} computer:///",
            glib::shell_quote(expected.to_str().unwrap()).to_string_lossy()
        );
        assert!(own_computer_shortcut(
            Path::new("Deze PC.desktop"),
            &exec,
            expected
        ));
        assert!(!own_computer_shortcut(
            Path::new("unrelated.desktop"),
            &exec,
            expected
        ));
        assert!(!own_computer_shortcut(
            Path::new("Deze PC.desktop"),
            &(exec.clone() + " --unexpected"),
            expected
        ));
        assert!(!own_computer_shortcut(
            Path::new("Deze PC.desktop"),
            "/usr/bin/false computer:///",
            expected
        ));
        assert!(!own_computer_shortcut(
            Path::new("Deze PC.desktop"),
            "'unterminated",
            expected
        ));
    }
    #[test]
    fn localized_type_names_and_icon_inheritance_are_preserved() {
        assert_eq!(type_name("text/plain", "name.txt", "nl"), "Tekstdocument");
        assert_eq!(type_name("image/png", "name.png", "en"), "PNG File");
        let f = Fixture::new();
        let bases = [f.0.clone()];
        for theme in ["Own", "Parent"] {
            fs::create_dir_all(f.0.join(theme).join("places")).unwrap();
        }
        fs::write(
            f.0.join("Own/index.theme"),
            "[Icon Theme]\nDirectories=places\nInherits=Parent\n[places]\nSize=32\n",
        )
        .unwrap();
        fs::write(
            f.0.join("Parent/index.theme"),
            "[Icon Theme]\nDirectories=places\nScaledDirectories=places@2\n[places]\nSize=32\n[places@2]\nSize=32\nScale=2\n",
        )
        .unwrap();
        fs::create_dir(f.0.join("Parent/places@2")).unwrap();
        fs::write(f.0.join("Parent/places@2/folder-download.svg"), b"<svg/>").unwrap();
        fs::write(f.0.join("Parent/places/user-home.svg"), b"<svg/>").unwrap();
        assert!(build_icon_catalog("Own", &bases).contains("user-home"));
        assert!(build_icon_catalog("Own", &bases).contains("folder-download"));
    }
}
