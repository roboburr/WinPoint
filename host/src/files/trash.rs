//! Freedesktop Trash operations and the original Explorer wire format.
//! Permanent removal is descriptor-relative and only visits validated Trash directories.
use super::{ApiError, ApiResult, Context, io_error, mutate, paths, properties::du};
use crate::i18n::Lang;
use gio::prelude::*;
use gtk::{gio, glib};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    ffi::{CString, OsStr, OsString},
    fs::{self, File, OpenOptions},
    hash::{Hash, Hasher},
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
        },
    },
    path::{Component, Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

const URI: &str = "trash:///";

#[derive(Clone, Debug)]
struct TrashDir {
    path: PathBuf,
    top: Option<PathBuf>,
}
#[derive(Clone, Debug)]
struct Entry {
    dir: TrashDir,
    name: String,
    path: PathBuf,
    original: Option<PathBuf>,
    deleted: i64,
}
struct Layout {
    home: PathBuf,
    mounts: Vec<PathBuf>,
    manual: bool,
    lock_root: PathBuf,
}

impl Layout {
    fn environment() -> Self {
        let overridden = std::env::var_os("WIN11SHELL_TRASH_DIR");
        Self {
            home: overridden
                .clone()
                .map(PathBuf::from)
                .unwrap_or_else(|| glib::user_data_dir().join("Trash")),
            mounts: if overridden.is_some() {
                Vec::new()
            } else {
                mount_points()
            },
            manual: overridden.is_some(),
            lock_root: glib::user_runtime_dir().join("win11-gnome/trash-locks"),
        }
    }
    fn dirs(&self) -> Vec<TrashDir> {
        let mut result = Vec::new();
        let mut seen = HashSet::new();
        let home = TrashDir {
            path: self.home.clone(),
            top: None,
        };
        if valid(&home) {
            seen.insert(fs::canonicalize(&home.path).unwrap_or_else(|_| home.path.clone()));
            result.push(home);
        }
        if self.manual {
            return result;
        }
        let uid = unsafe { libc::getuid() };
        for mount in &self.mounts {
            for path in [
                mount.join(".Trash").join(uid.to_string()),
                mount.join(format!(".Trash-{uid}")),
            ] {
                let candidate = TrashDir {
                    path,
                    top: Some(mount.clone()),
                };
                if !valid(&candidate) {
                    continue;
                }
                if candidate
                    .path
                    .parent()
                    .is_some_and(|path| path.file_name() == Some(OsStr::new(".Trash")))
                {
                    let Ok(parent) = fs::symlink_metadata(candidate.path.parent().unwrap()) else {
                        continue;
                    };
                    if !parent.is_dir()
                        || parent.file_type().is_symlink()
                        || parent.mode() & libc::S_ISVTX == 0
                    {
                        continue;
                    }
                }
                let key =
                    fs::canonicalize(&candidate.path).unwrap_or_else(|_| candidate.path.clone());
                if seen.insert(key) {
                    result.push(candidate);
                }
            }
        }
        result
    }
    fn entries(&self) -> Vec<Entry> {
        let mut entries = Vec::new();
        for dir in self.dirs() {
            let Ok(files) = fs::read_dir(dir.path.join("files")) else {
                continue;
            };
            for file in files.flatten() {
                let Ok(name) = file.file_name().into_string() else {
                    continue;
                };
                let (original, deleted) =
                    parse_info(&dir.path.join("info").join(format!("{name}.trashinfo")));
                let original = original.and_then(|path| original_path(&path, dir.top.as_deref()));
                entries.push(Entry {
                    path: file.path(),
                    dir: dir.clone(),
                    name,
                    original,
                    deleted,
                });
            }
        }
        entries
    }
    fn lock(&self, lang: Lang) -> Result<TrashLock, ApiError> {
        crate::events::private_dir(&self.lock_root)
            .map_err(|error| io_error(error, &self.lock_root, lang))?;
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        self.home.hash(&mut hash);
        let path = self.lock_root.join(format!("{:016x}.lock", hash.finish()));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
            .map_err(|error| io_error(error, &path, lang))?;
        let metadata = file
            .metadata()
            .map_err(|error| io_error(error, &path, lang))?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
            || metadata.nlink() != 1
        {
            return Err(ApiError::tr(
                "no_access",
                lang,
                "files.trashLockNotPrivate",
                crate::i18n::NO_PARAMS,
            ));
        }
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
                return Ok(TrashLock(file));
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(io_error(error, &path, lang));
            }
        }
    }
}
struct TrashLock(File);
impl Drop for TrashLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

pub(super) fn call(method: &str, args: &Value, ctx: &Context) -> Option<ApiResult> {
    let layout = Layout::environment();
    match method {
        "files.list"
            if args["path"]
                .as_str()
                .is_some_and(|path| path.starts_with("trash:")) =>
        {
            Some(list_in(&layout, ctx))
        }
        "files.trash" => Some(trash(&layout, args, ctx)),
        "files.trashInfo" => Some(info(&layout)),
        "files.emptyTrash" => Some(empty(&layout, ctx)),
        "files.restoreTrash" => Some(restore(&layout, args, ctx)),
        _ => None,
    }
}
pub(super) fn count() -> usize {
    Layout::environment()
        .dirs()
        .into_iter()
        .map(|dir| {
            fs::read_dir(dir.path.join("files"))
                .map(|entries| entries.count())
                .unwrap_or(0)
        })
        .sum()
}

pub(super) fn watch_dirs() -> Vec<PathBuf> {
    Layout::environment()
        .dirs()
        .into_iter()
        .map(|dir| dir.path.join("files"))
        .collect()
}
pub(super) fn home_watch_dir() -> PathBuf {
    Layout::environment().home.join("files")
}

pub(super) fn list(ctx: &Context) -> ApiResult {
    list_in(&Layout::environment(), ctx)
}

fn own_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && metadata.uid() == unsafe { libc::getuid() }
    })
}
fn valid(dir: &TrashDir) -> bool {
    (dir.top.is_none() || own_dir(&dir.path))
        && own_dir(&dir.path.join("files"))
        && (fs::symlink_metadata(dir.path.join("info"))
            .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
            || own_dir(&dir.path.join("info")))
}
fn mount_points() -> Vec<PathBuf> {
    const TYPES: &[&str] = &[
        "ext2", "ext3", "ext4", "xfs", "btrfs", "f2fs", "jfs", "reiserfs", "zfs", "ntfs", "ntfs3",
        "fuseblk", "exfat", "vfat", "msdos", "hfsplus", "bcachefs", "nilfs2",
    ];
    fs::read_to_string("/proc/self/mounts")
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() < 3 || !TYPES.contains(&fields[2]) {
                return None;
            }
            Some(PathBuf::from(
                fields[1]
                    .replace("\\040", " ")
                    .replace("\\011", "\t")
                    .replace("\\134", "\\"),
            ))
        })
        .collect()
}
fn normalized(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                result.pop();
            }
            Component::CurDir => {}
            _ => result.push(part.as_os_str()),
        }
    }
    result
}
fn original_path(path: &str, top: Option<&Path>) -> Option<PathBuf> {
    if path.is_empty() || path.contains('\0') {
        return None;
    }
    let raw = Path::new(path);
    if let Some(top) = top {
        let full = normalized(&if raw.is_absolute() {
            raw.into()
        } else {
            top.join(raw)
        });
        let top = normalized(top);
        if full != top && full.starts_with(&top) {
            Some(full)
        } else {
            None
        }
    } else if raw.is_absolute() {
        Some(normalized(raw))
    } else {
        None
    }
}
fn percent_decode(value: &str) -> String {
    let input = value.as_bytes();
    let mut out = Vec::with_capacity(input.len());
    let mut at = 0;
    while at < input.len() {
        if input[at] == b'%'
            && at + 2 < input.len()
            && let (Some(a), Some(b)) = (
                (input[at + 1] as char).to_digit(16),
                (input[at + 2] as char).to_digit(16),
            )
        {
            out.push((a * 16 + b) as u8);
            at += 3;
        } else {
            out.push(input[at]);
            at += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
fn percent_encode(path: &Path) -> String {
    let mut out = String::new();
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'_' | b'-' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
fn parse_info(path: &Path) -> (Option<String>, i64) {
    let Ok(file) = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
    else {
        return (None, 0);
    };
    if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        return (None, 0);
    }
    let mut bytes = Vec::new();
    if file.take(1024 * 1024 + 1).read_to_end(&mut bytes).is_err() || bytes.len() > 1024 * 1024 {
        return (None, 0);
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut section = false;
    let mut original = None;
    let mut deleted = 0;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            section = line == "[Trash Info]";
            continue;
        }
        if !section {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "Path" => original = Some(percent_decode(value)),
            "DeletionDate" => {
                if let Ok(date) =
                    glib::DateTime::from_iso8601(value, Some(&glib::TimeZone::local()))
                {
                    deleted =
                        date.to_unix().saturating_mul(1000) + i64::from(date.microsecond() / 1000);
                }
            }
            _ => {}
        }
    }
    (original, deleted)
}

fn list_in(layout: &Layout, ctx: &Context) -> ApiResult {
    let mut items = Vec::new();
    for entry in layout.entries() {
        let Ok(mut item) = ctx.item(&entry.path) else {
            continue;
        };
        let original = entry
            .original
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !original.is_empty() && item["isDesktopEntry"] != true {
            item["displayName"] = json!(
                entry
                    .original
                    .as_ref()
                    .and_then(|path| path.file_name())
                    .unwrap_or_default()
                    .to_string_lossy()
            );
        }
        item["hidden"] = json!(false);
        item["originalPath"] = json!(original);
        item["originalDir"] = json!(
            entry
                .original
                .as_ref()
                .map(|path| path.parent().unwrap_or(Path::new("/")))
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default()
        );
        item["deletedAt"] = json!(entry.deleted);
        item["trashName"] = json!(entry.name);
        item["inTrash"] = json!(true);
        items.push(item);
    }
    items.sort_by(|a, b| {
        b["deletedAt"]
            .as_i64()
            .unwrap_or(0)
            .cmp(&a["deletedAt"].as_i64().unwrap_or(0))
    });
    Ok(json!({"path":URI,"parent":null,"items":items,"writable":false,"trash":true}))
}

type Fingerprint = (PathBuf, Option<(u64, u64, u64, i64, i64)>);
type Cache = HashMap<(PathBuf, Vec<Fingerprint>), (Instant, Value)>;
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
pub(super) fn invalidate() {
    if let Ok(mut cache) = CACHE.get_or_init(|| Mutex::new(HashMap::new())).lock() {
        cache.clear();
    }
}
fn info(layout: &Layout) -> ApiResult {
    // Counts/bytes include names which cannot be represented by the UTF-8 UI.
    let entries: Vec<PathBuf> = layout
        .dirs()
        .into_iter()
        .flat_map(|dir| {
            fs::read_dir(dir.path.join("files"))
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| entry.path())
        })
        .collect();
    let mut identities: Vec<Fingerprint> = entries
        .iter()
        .map(|path| {
            let identity = fs::symlink_metadata(path).ok().map(|metadata| {
                (
                    metadata.dev(),
                    metadata.ino(),
                    metadata.len(),
                    metadata.mtime(),
                    metadata.mtime_nsec(),
                )
            });
            (path.clone(), identity)
        })
        .collect();
    identities.sort_by(|left, right| left.0.cmp(&right.0));
    let key = (layout.home.clone(), identities);
    if let Ok(cache) = CACHE.get_or_init(|| Mutex::new(HashMap::new())).lock()
        && let Some((at, value)) = cache.get(&key)
        && at.elapsed() < Duration::from_secs(30)
    {
        return Ok(value.clone());
    }
    let deadline = Instant::now() + Duration::from_millis(1500);
    let mut size = 0_u64;
    let mut complete = true;
    for entry in &entries {
        let (bytes, _, _, done) = du(entry, deadline);
        size = size.saturating_add(bytes);
        complete &= done;
        if !done {
            break;
        }
    }
    let value = json!({"count":entries.len(),"size":size,"approx":!complete});
    if let Ok(mut cache) = CACHE.get_or_init(|| Mutex::new(HashMap::new())).lock() {
        if cache.len() > 32 {
            cache.clear();
        }
        cache.insert(key, (Instant::now(), value.clone()));
    }
    Ok(value)
}

fn manual_one(layout: &Layout, source: &Path, lang: Lang) -> Result<String, ApiError> {
    for child in [
        &layout.home,
        &layout.home.join("files"),
        &layout.home.join("info"),
    ] {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(child)
            .map_err(|error| io_error(error, child, lang))?;
    }
    let dir = TrashDir {
        path: layout.home.clone(),
        top: None,
    };
    if !valid(&dir) {
        return Err(ApiError::tr(
            "no_access",
            lang,
            "files.trashUnsafe",
            crate::i18n::NO_PARAMS,
        ));
    }
    let base = source
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| ApiError::tr("bad_args", lang, "files.badPath", crate::i18n::NO_PARAMS))?;
    let root = open_root(&dir).map_err(|error| io_error(error, source, lang))?;
    let files = open_owned_child(&root, OsStr::new("files"))
        .map_err(|error| io_error(error, source, lang))?;
    let info = open_owned_child(&root, OsStr::new("info"))
        .map_err(|error| io_error(error, source, lang))?;
    let files_path = PathBuf::from(format!("/proc/self/fd/{}", files.as_raw_fd()));
    let info_path = PathBuf::from(format!("/proc/self/fd/{}", info.as_raw_fd()));
    let mut preferred = base.to_owned();
    for _ in 0..50 {
        let target = mutate::unique_path(&files_path, &preferred, false)
            .map_err(|error| io_error(error, source, lang))?;
        let name = target.file_name().unwrap().to_string_lossy().into_owned();
        let path = info_path.join(format!("{name}.trashinfo"));
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                // An orphaned info file is not a source item, but never follow it or
                // overwrite another concurrent trash operation's metadata.
                preferred = next_name(&name);
                continue;
            }
            Err(error) => return Err(io_error(error, source, lang)),
        };
        let date = glib::DateTime::now_local()
            .and_then(|date| date.format("%Y-%m-%dT%H:%M:%S"))
            .map(|date| date.to_string())
            .unwrap_or_else(|_| "1970-01-01T00:00:00".into());
        let text = format!(
            "[Trash Info]\nPath={}\nDeletionDate={date}\n",
            percent_encode(source)
        );
        if let Err(error) = file
            .write_all(text.as_bytes())
            .and_then(|_| file.sync_all())
        {
            let _ = fs::remove_file(&path);
            return Err(io_error(error, source, lang));
        }
        match mutate::rename_noreplace(source, &target) {
            Ok(()) => return Ok(name),
            Err(error) => {
                let _ = fs::remove_file(path);
                if error.kind() != io::ErrorKind::AlreadyExists {
                    return Err(io_error(error, source, lang));
                }
            }
        }
    }
    Err(ApiError::tr(
        "exists",
        lang,
        "files.exists",
        crate::i18n::NO_PARAMS,
    ))
}

fn next_name(name: &str) -> String {
    let low = name.to_ascii_lowercase();
    let compound = [
        ".tar.gz",
        ".tar.xz",
        ".tar.bz2",
        ".tar.zst",
        ".tar.lz",
        ".tar.lzma",
    ]
    .into_iter()
    .find(|extension| low.ends_with(extension));
    let split = compound
        .map(|extension| name.len() - extension.len())
        .or_else(|| {
            name.rfind('.')
                .filter(|index| name[..*index].chars().any(|character| character != '.'))
        });
    let (stem, extension) = split
        .map(|index| name.split_at(index))
        .unwrap_or((name, ""));
    if stem.ends_with(')')
        && let Some(index) = stem.rfind(" (")
        && let Ok(number) = stem[index + 2..stem.len() - 1].parse::<u64>()
    {
        format!(
            "{} ({}){extension}",
            &stem[..index],
            number.saturating_add(1)
        )
    } else {
        format!("{stem} (2){extension}")
    }
}

fn mount_top(path: &Path, device: u64) -> PathBuf {
    let mut current = path.parent().unwrap_or(Path::new("/")).to_path_buf();
    while current != Path::new("/") {
        let parent = current.parent().unwrap_or(Path::new("/"));
        if !fs::metadata(parent).is_ok_and(|metadata| metadata.dev() == device) {
            return current;
        }
        current = parent.into();
    }
    current
}
fn dirs_for(layout: &Layout, sources: &[PathBuf]) -> Vec<TrashDir> {
    let home_dev = fs::metadata(if layout.home.exists() {
        layout.home.as_path()
    } else {
        layout.home.parent().unwrap_or(Path::new("/"))
    })
    .ok()
    .map(|metadata| metadata.dev());
    let uid = unsafe { libc::getuid() };
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for source in sources {
        let Ok(metadata) = fs::symlink_metadata(source) else {
            continue;
        };
        if layout.manual || Some(metadata.dev()) == home_dev {
            if seen.insert(layout.home.clone()) {
                result.push(TrashDir {
                    path: layout.home.clone(),
                    top: None,
                });
            }
        } else {
            let top = mount_top(source, metadata.dev());
            for path in [
                top.join(".Trash").join(uid.to_string()),
                top.join(format!(".Trash-{uid}")),
            ] {
                if seen.insert(path.clone()) {
                    result.push(TrashDir {
                        path,
                        top: Some(top.clone()),
                    });
                }
            }
        }
    }
    result
}
fn info_names(dir: &TrashDir) -> HashSet<OsString> {
    fs::read_dir(dir.path.join("info"))
        .map(|entries| entries.flatten().map(|entry| entry.file_name()).collect())
        .unwrap_or_default()
}
fn gio_one(path: &Path, lang: Lang) -> Result<(), ApiError> {
    gio::File::for_path(path)
        .trash(gio::Cancellable::NONE)
        .map_err(|error| {
            if error.matches(gio::IOErrorEnum::NotSupported) {
                ApiError::tr("no_trash", lang, "files.noTrash", crate::i18n::NO_PARAMS)
            } else if error.matches(gio::IOErrorEnum::PermissionDenied) {
                ApiError::tr(
                    "no_access",
                    lang,
                    "files.noDeleteAccess",
                    crate::i18n::NO_PARAMS,
                )
            } else if error.matches(gio::IOErrorEnum::NotFound) {
                ApiError::tr("not_found", lang, "files.itemGone", crate::i18n::NO_PARAMS)
            } else {
                let mut result =
                    ApiError::tr("failed", lang, "files.trashFailed", crate::i18n::NO_PARAMS);
                result.data = Some(json!(error.message()));
                result
            }
        })
}
fn trash(layout: &Layout, args: &Value, ctx: &Context) -> ApiResult {
    let lang = ctx.lang();
    let sources = paths(&args["paths"], lang)?;
    let _lock = layout.lock(lang)?;
    let before: Vec<_> = if layout.manual {
        Vec::new()
    } else {
        dirs_for(layout, &sources)
            .into_iter()
            .map(|dir| {
                let names = info_names(&dir);
                (dir, names)
            })
            .collect()
    };
    let mut trashed = Vec::new();
    let mut failed = Vec::new();
    let mut unknown = HashSet::new();
    for source in &sources {
        let result = if fs::symlink_metadata(source).is_err() {
            Err(ApiError::tr(
                "not_found",
                lang,
                "files.itemGone",
                crate::i18n::NO_PARAMS,
            ))
        } else if layout.manual {
            manual_one(layout, source, lang).map(Some)
        } else {
            gio_one(source, lang).map(|_| None)
        };
        match result {Ok(Some(name))=>trashed.push(json!({"path":source,"name":name,"trashPath":layout.home.join("files").join(&name)})),
            Ok(None)=>{unknown.insert(source.clone());},Err(error)=>failed.push(json!({"path":source,"error":error.message,"code":error.code}))}
    }
    if !unknown.is_empty() {
        let mut found: HashMap<PathBuf, (i64, Value)> = HashMap::new();
        for (dir, old) in before {
            for name in info_names(&dir).difference(&old) {
                let Some(name) = name
                    .to_str()
                    .and_then(|name| name.strip_suffix(".trashinfo"))
                else {
                    continue;
                };
                let (original, deleted) =
                    parse_info(&dir.path.join("info").join(format!("{name}.trashinfo")));
                let Some(original) =
                    original.and_then(|path| original_path(&path, dir.top.as_deref()))
                else {
                    continue;
                };
                if unknown.contains(&original)
                    && found.get(&original).is_none_or(|(at, _)| deleted >= *at)
                {
                    found.insert(original.clone(),(deleted,json!({"path":original,"name":name,"trashPath":dir.path.join("files").join(name)})));
                }
            }
        }
        for source in &sources {
            if let Some((_, value)) = found.remove(source) {
                trashed.push(value);
            }
        }
    }
    invalidate();
    for source in &sources {
        if let Some(parent) = source.parent() {
            ctx.changed(parent);
        }
    }
    ctx.emit("files.trashChanged", json!({}));
    Ok(json!({"ok":failed.is_empty(),"failed":failed,"trashed":trashed}))
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::String(value) => !value.is_empty(),
        Value::Number(value) => value.as_f64() != Some(0.0),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    }
}
fn inside(child: &Path, parent: &Path) -> bool {
    // canonicalize the longest existing prefix, like os.path.realpath(strict=False).
    fn real(path: &Path) -> Option<PathBuf> {
        if let Ok(real) = fs::canonicalize(path) {
            return Some(real);
        }
        let parent = path.parent()?;
        let name = path.file_name()?;
        Some(real(parent)?.join(name))
    }
    real(child)
        .zip(real(parent))
        .is_some_and(|(child, parent)| child == parent || child.starts_with(parent))
}
fn restore(layout: &Layout, args: &Value, ctx: &Context) -> ApiResult {
    enum Wanted {
        Item(Entry),
        Missing(String),
    }
    let lang = ctx.lang();
    let _lock = layout.lock(lang)?;
    let entries = layout.entries();
    let mut wanted = Vec::new();
    let mut failed = Vec::new();
    if truthy(&args["all"]) {
        wanted = entries.iter().cloned().map(Wanted::Item).collect();
    } else {
        let value = if truthy(&args["names"]) {
            &args["names"]
        } else {
            &args["paths"]
        };
        let names: Vec<&Value> = if value.is_string() {
            vec![value]
        } else if let Some(names) = value.as_array() {
            names.iter().collect()
        } else {
            Vec::new()
        };
        if names.is_empty() {
            return Err(ApiError::tr(
                "bad_args",
                lang,
                "files.noItems",
                crate::i18n::NO_PARAMS,
            ));
        }
        let mut ordered = entries.clone();
        ordered.sort_by_key(|entry| entry.dir.top.is_some());
        for name in names {
            let Some(name) = name.as_str() else { continue };
            let found = if name.starts_with('/') {
                let path = normalized(Path::new(name));
                entries.iter().find(|entry| entry.path == path)
            } else {
                ordered.iter().find(|entry| entry.name == name)
            };
            if let Some(entry) = found {
                wanted.push(Wanted::Item(entry.clone()));
            } else {
                wanted.push(Wanted::Missing(name.into()));
            }
        }
    }
    let mut restored = Vec::new();
    let mut touched = HashSet::new();
    for wanted in wanted {
        let entry = match wanted {
            Wanted::Item(entry) => entry,
            Wanted::Missing(name) => {
                failed.push(json!({"name":name,"error":crate::i18n::tr(lang, "files.notInTrash")}));
                continue;
            }
        };
        let outside = || crate::i18n::tr(lang, "files.otherDrive");
        let Some(original) = entry.original.as_ref() else {
            failed.push(json!({"name":entry.name,"error":if entry.dir.top.is_some(){outside()}else{crate::i18n::tr(lang, "files.unknownOrigin")}}));
            continue;
        };
        let parent = original.parent().unwrap_or(Path::new("/"));
        if entry
            .dir
            .top
            .as_ref()
            .is_some_and(|top| !inside(parent, top))
        {
            failed.push(json!({"name":entry.name,"error":outside()}));
            continue;
        }
        let result = (|| {
            // Revalidate the trash directory just before accessing its stored item.
            if !valid(&entry.dir) {
                return Err(io::Error::from_raw_os_error(libc::EACCES));
            }
            let root = open_root(&entry.dir)?;
            let files = open_owned_child(&root, OsStr::new("files"))?;
            let info = open_owned_child(&root, OsStr::new("info")).ok();
            let source =
                PathBuf::from(format!("/proc/self/fd/{}", files.as_raw_fd())).join(&entry.name);
            let parent_fd = make_parent(parent, entry.dir.top.as_deref())?;
            let parent_path = PathBuf::from(format!("/proc/self/fd/{}", parent_fd.as_raw_fd()));
            let is_dir = fs::symlink_metadata(&source)?.is_dir();
            let name = original
                .file_name()
                .and_then(OsStr::to_str)
                .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?;
            for _ in 0..20 {
                let target = mutate::unique_path(&parent_path, name, is_dir)?;
                match mutate::move_noreplace_exact(&source, &target) {
                    Ok(()) => {
                        if let Some(info) = &info {
                            let _ = unlink_at(
                                info,
                                OsStr::new(&format!("{}.trashinfo", entry.name)),
                                false,
                            );
                        }
                        return Ok(parent.join(target.file_name().unwrap()));
                    }
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => return Err(error),
                }
            }
            Err(io::Error::from_raw_os_error(libc::EEXIST))
        })();
        match result {
            Ok(target) => {
                restored.push(target);
                touched.insert(parent.to_path_buf());
            }
            Err(error) => failed
                .push(json!({"name":entry.name,"error":io_error(error, original, lang).message})),
        }
    }
    invalidate();
    for path in touched {
        ctx.changed(&path);
    }
    ctx.emit("files.trashChanged", json!({}));
    Ok(json!({"ok":failed.is_empty(),"restored":restored,"failed":failed}))
}

fn c_name(name: &OsStr) -> io::Result<CString> {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes == b"." || bytes == b".." || bytes.contains(&b'/') {
        return Err(io::Error::from_raw_os_error(libc::EINVAL));
    }
    CString::new(bytes).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))
}
fn open_at(parent: &File, name: &OsStr, flags: i32) -> io::Result<File> {
    let name = c_name(name)?;
    let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags | libc::O_CLOEXEC) };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}
fn stat_at(parent: &File, name: &OsStr) -> io::Result<libc::stat> {
    let name = c_name(name)?;
    let mut result = std::mem::MaybeUninit::uninit();
    if unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            name.as_ptr(),
            result.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } < 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { result.assume_init() })
    }
}
fn names_at(directory: &File) -> io::Result<Vec<OsString>> {
    fs::read_dir(format!("/proc/self/fd/{}", directory.as_raw_fd()))?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect()
}
fn unlink_at(parent: &File, name: &OsStr, directory: bool) -> io::Result<()> {
    let name = c_name(name)?;
    if unsafe {
        libc::unlinkat(
            parent.as_raw_fd(),
            name.as_ptr(),
            if directory { libc::AT_REMOVEDIR } else { 0 },
        )
    } < 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
fn open_owned_child(parent: &File, name: &OsStr) -> io::Result<File> {
    let file = open_at(
        parent,
        name,
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW,
    )?;
    if file.metadata()?.uid() != unsafe { libc::getuid() } {
        return Err(io::Error::from_raw_os_error(libc::EACCES));
    }
    Ok(file)
}
fn open_root(dir: &TrashDir) -> io::Result<File> {
    // Resolve a deliberately relocated home Trash once, then walk the physical path
    // through pinned descriptors. A later swapped component cannot redirect deletion.
    let path = if dir.top.is_none() {
        fs::canonicalize(&dir.path)?
    } else {
        dir.path.clone()
    };
    let mut opened = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open("/")?;
    for part in path.components() {
        if let Component::Normal(name) = part {
            opened = open_at(
                &opened,
                name,
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW,
            )?;
        }
    }
    if dir.top.is_some() && opened.metadata()?.uid() != unsafe { libc::getuid() } {
        return Err(io::Error::from_raw_os_error(libc::EACCES));
    }
    Ok(opened)
}
fn make_parent(path: &Path, top: Option<&Path>) -> io::Result<File> {
    fn real(path: &Path) -> io::Result<PathBuf> {
        match fs::canonicalize(path) {
            Ok(path) => Ok(path),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let parent = path
                    .parent()
                    .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?;
                let name = path
                    .file_name()
                    .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?;
                Ok(real(parent)?.join(name))
            }
            Err(error) => Err(error),
        }
    }
    let physical = real(path)?;
    if let Some(top) = top {
        if !physical.starts_with(fs::canonicalize(top)?) {
            return Err(io::Error::from_raw_os_error(libc::EACCES));
        }
    }
    let mut opened = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open("/")?;
    for component in physical.components() {
        if let Component::Normal(name) = component {
            match open_at(
                &opened,
                name,
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW,
            ) {
                Ok(next) => opened = next,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    let name_c = c_name(name)?;
                    if unsafe { libc::mkdirat(opened.as_raw_fd(), name_c.as_ptr(), 0o777) } < 0 {
                        let error = io::Error::last_os_error();
                        if error.kind() != io::ErrorKind::AlreadyExists {
                            return Err(error);
                        }
                    }
                    opened = open_at(
                        &opened,
                        name,
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW,
                    )?;
                }
                Err(error) => return Err(error),
            }
        }
    }
    Ok(opened)
}
fn mount_id(file: &File) -> io::Result<u64> {
    let info = fs::read_to_string(format!("/proc/self/fdinfo/{}", file.as_raw_fd()))?;
    info.lines()
        .find_map(|line| line.strip_prefix("mnt_id:").map(str::trim))
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| io::Error::from_raw_os_error(libc::EOPNOTSUPP))
}
fn same_mount(
    parent: &File,
    child: &File,
    mounts: &impl Fn(&File) -> io::Result<u64>,
) -> io::Result<()> {
    if parent.metadata()?.dev() != child.metadata()?.dev() || mounts(parent)? != mounts(child)? {
        return Err(io::Error::from_raw_os_error(libc::EBUSY));
    }
    Ok(())
}
fn directory_for_removal(
    parent: &File,
    name: &OsStr,
    metadata: &libc::stat,
    mounts: &impl Fn(&File) -> io::Result<u64>,
) -> io::Result<File> {
    let held = open_at(
        parent,
        name,
        libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW,
    )?;
    let held_meta = held.metadata()?;
    if held_meta.dev() != metadata.st_dev || held_meta.ino() != metadata.st_ino {
        return Err(io::Error::from_raw_os_error(libc::EBUSY));
    }
    // A bind mount can share st_dev, so compare the kernel mount ID as well.
    // Check before repairing permissions: external mounted data must remain untouched.
    same_mount(parent, &held, mounts)?;
    if held_meta.uid() == unsafe { libc::getuid() } && held_meta.mode() & 0o700 != 0o700 {
        let path = CString::new(format!("/proc/self/fd/{}", held.as_raw_fd())).unwrap();
        if unsafe { libc::chmod(path.as_ptr(), (held_meta.mode() | 0o700) & 0o7777) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    let directory = open_at(
        parent,
        name,
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW,
    )?;
    let actual = directory.metadata()?;
    if actual.dev() != held_meta.dev() || actual.ino() != held_meta.ino() {
        return Err(io::Error::from_raw_os_error(libc::EBUSY));
    }
    same_mount(parent, &directory, mounts)?;
    Ok(directory)
}
fn check_tree_mounts(
    parent: &File,
    name: &OsStr,
    depth: usize,
    mounts: &impl Fn(&File) -> io::Result<u64>,
) -> io::Result<()> {
    let metadata = stat_at(parent, name)?;
    if metadata.st_mode & libc::S_IFMT != libc::S_IFDIR {
        let held = open_at(parent, name, libc::O_PATH | libc::O_NOFOLLOW)?;
        let actual = held.metadata()?;
        if actual.dev() != metadata.st_dev || actual.ino() != metadata.st_ino {
            return Err(io::Error::from_raw_os_error(libc::EBUSY));
        }
        return same_mount(parent, &held, mounts);
    }
    if depth > 1024 {
        return Err(io::Error::from_raw_os_error(libc::ELOOP));
    }
    let directory = directory_for_removal(parent, name, &metadata, mounts)?;
    for child in names_at(&directory)? {
        check_tree_mounts(&directory, &child, depth + 1, mounts)?;
    }
    Ok(())
}
fn remove_at(parent: &File, name: &OsStr, depth: usize) -> io::Result<()> {
    let metadata = stat_at(parent, name)?;
    if metadata.st_mode & libc::S_IFMT != libc::S_IFDIR {
        return unlink_at(parent, name, false);
    }
    if depth > 1024 {
        return Err(io::Error::from_raw_os_error(libc::ELOOP));
    }
    // Recheck during deletion as a mount could have appeared after the preflight.
    let directory = directory_for_removal(parent, name, &metadata, &mount_id)?;
    for child in names_at(&directory)? {
        remove_at(&directory, &child, depth + 1)?;
    }
    let current = stat_at(parent, name)?;
    if current.st_dev != metadata.st_dev || current.st_ino != metadata.st_ino {
        return Err(io::Error::from_raw_os_error(libc::EBUSY));
    }
    unlink_at(parent, name, true)
}
fn empty(layout: &Layout, ctx: &Context) -> ApiResult {
    empty_with_mounts(layout, ctx, &mount_id)
}
fn empty_with_mounts(
    layout: &Layout,
    ctx: &Context,
    mounts: &impl Fn(&File) -> io::Result<u64>,
) -> ApiResult {
    let lang = ctx.lang();
    let _lock = layout.lock(lang)?;
    let mut removed = 0_u64;
    let mut failed = 0_u64;
    for dir in layout.dirs() {
        if !valid(&dir) {
            continue;
        }
        let Ok(root) = open_root(&dir) else { continue };
        let Ok(files) = open_owned_child(&root, OsStr::new("files")) else {
            continue;
        };
        if same_mount(&root, &files, mounts).is_err() {
            failed += 1;
            continue;
        }
        let info = match open_owned_child(&root, OsStr::new("info")) {
            Ok(info) if same_mount(&root, &info, mounts).is_ok() => Some(info),
            Ok(_) => {
                // Metadata is deleted too: a separately mounted info directory is
                // outside this Trash transaction, even if its owner happens to match.
                failed += 1;
                continue;
            }
            Err(_) => None,
        };
        if let Ok(names) = names_at(&files) {
            for name in names {
                // Inspect the complete selected tree before removing its first file.
                // An embedded mount makes this one entry fail without a partial tree.
                match check_tree_mounts(&files, &name, 0, mounts)
                    .and_then(|()| remove_at(&files, &name, 0))
                {
                    Ok(()) => {
                        removed += 1;
                        if let Some(info) = &info {
                            let mut info_name = name.clone();
                            info_name.push(".trashinfo");
                            let _ = unlink_at(info, &info_name, false);
                        }
                    }
                    Err(_) => failed += 1,
                }
            }
        }
        if let Some(info) = &info
            && let Ok(names) = names_at(info)
        {
            for name in names {
                let Some(stem) = name.as_bytes().strip_suffix(b".trashinfo") else {
                    continue;
                };
                if stem.is_empty()
                    || stat_at(&files, OsStr::from_bytes(stem))
                        .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
                {
                    let _ = unlink_at(info, &name, false);
                }
            }
        }
        let _ = unlink_at(&root, OsStr::new("directorysizes"), false);
        if let Ok(expunged) = open_owned_child(&root, OsStr::new("expunged"))
            && same_mount(&root, &expunged, mounts).is_ok()
            && let Ok(names) = names_at(&expunged)
        {
            for name in names {
                let _ = check_tree_mounts(&expunged, &name, 0, mounts)
                    .and_then(|()| remove_at(&expunged, &name, 0));
            }
        }
    }
    invalidate();
    ctx.emit("files.trashChanged", json!({}));
    ctx.changed(&layout.home.join("files"));
    if failed > 0 {
        let mut error = ApiError::trn(
            "partial",
            lang,
            "files.emptyTrashFailed",
            failed,
            crate::i18n::NO_PARAMS,
        );
        error.data = Some(json!({"removed":removed}));
        Err(error)
    } else {
        Ok(json!({"ok":true,"removed":removed}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    struct Temp {
        root: PathBuf,
    }
    impl Temp {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("win11-trash-{}", crate::events::nonce()));
            fs::create_dir(&root).unwrap();
            Self { root }
        }
        fn layout(&self) -> Layout {
            Layout {
                home: self.root.join("Trash"),
                mounts: Vec::new(),
                manual: true,
                lock_root: self.root.join("locks"),
            }
        }
        fn ctx(&self) -> Context {
            Context::new(
                self.root.clone(),
                self.root.join("Desktop"),
                "en".into(),
                false,
            )
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    #[test]
    fn trash_list_info_restore_conflict_and_undo_names_keep_original_contract() {
        let t = Temp::new();
        let l = t.layout();
        let c = t.ctx();
        let source = t.root.join("spatie é %?.txt");
        fs::write(&source, b"hello").unwrap();
        let result = trash(&l, &json!({"paths":[source]}), &c).unwrap();
        assert_eq!(result["ok"], true);
        assert!(!source.exists());
        let name = result["trashed"][0]["name"].as_str().unwrap();
        let listing = list_in(&l, &c).unwrap();
        let item = &listing["items"][0];
        assert_eq!(item["originalPath"], json!(source));
        assert_eq!(item["trashName"], name);
        assert_eq!(item["hidden"], false);
        assert_eq!(info(&l).unwrap()["size"], 5);
        fs::write(&source, b"occupied").unwrap();
        let restored = restore(&l, &json!({"names":name}), &c).unwrap();
        assert_eq!(restored["ok"], true);
        assert_eq!(fs::read(&source).unwrap(), b"occupied");
        assert_eq!(
            fs::read(restored["restored"][0].as_str().unwrap()).unwrap(),
            b"hello"
        );
        assert_eq!(info(&l).unwrap()["count"], 0);
    }
    #[test]
    fn trash_preserves_broken_links_and_partial_failures() {
        let t = Temp::new();
        let l = t.layout();
        let c = t.ctx();
        let link = t.root.join("broken");
        symlink("missing", &link).unwrap();
        let result = trash(&l, &json!({"paths":[link,t.root.join("absent")]}), &c).unwrap();
        assert_eq!(result["ok"], false);
        assert_eq!(result["failed"][0]["code"], "not_found");
        let restored = restore(&l, &json!({"all":true}), &c).unwrap();
        assert_eq!(restored["ok"], true);
        assert_eq!(fs::read_link(link).unwrap(), PathBuf::from("missing"));
    }
    #[test]
    fn permanent_empty_does_not_follow_entry_links_or_linked_files_directory() {
        let t = Temp::new();
        let l = t.layout();
        let c = t.ctx();
        let outside = t.root.join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep"), b"sentinel").unwrap();
        fs::create_dir_all(l.home.join("files")).unwrap();
        fs::create_dir(l.home.join("info")).unwrap();
        symlink(&outside, l.home.join("files/link")).unwrap();
        assert_eq!(empty(&l, &c).unwrap()["removed"], 1);
        assert_eq!(fs::read(outside.join("keep")).unwrap(), b"sentinel");
        fs::remove_dir(l.home.join("files")).unwrap();
        symlink(&outside, l.home.join("files")).unwrap();
        assert_eq!(empty(&l, &c).unwrap()["removed"], 0);
        assert!(outside.join("keep").exists());
    }
    #[test]
    fn mounted_child_preflight_keeps_entire_tree_and_reports_partial_success() {
        let t = Temp::new();
        let l = t.layout();
        let c = t.ctx();
        let tree = l.home.join("files/tree");
        let mounted = tree.join("mounted");
        fs::create_dir_all(&mounted).unwrap();
        fs::create_dir(l.home.join("info")).unwrap();
        fs::write(tree.join("keep"), b"keep sibling").unwrap();
        fs::write(mounted.join("external"), b"mounted data").unwrap();
        fs::write(l.home.join("files/removable"), b"remove me").unwrap();
        let mounted_inode = fs::metadata(&mounted).unwrap().ino();
        fs::set_permissions(&mounted, fs::Permissions::from_mode(0)).unwrap();
        let error = empty_with_mounts(&l, &c, &|file| {
            let real = mount_id(file)?;
            Ok(real + u64::from(file.metadata()?.ino() == mounted_inode))
        })
        .unwrap_err();
        assert_eq!(error.code, "partial");
        // the test context is English (D-G1a: one item is singular now)
        assert_eq!(error.message, "1 item couldn’t be deleted.");
        assert_eq!(error.json()["key"], "files.emptyTrashFailed");
        assert_eq!(error.json()["params"], json!({"n":"1"}));
        assert_eq!(error.data.unwrap()["removed"], 1);
        assert_eq!(fs::read(tree.join("keep")).unwrap(), b"keep sibling");
        assert_eq!(fs::metadata(&mounted).unwrap().mode() & 0o777, 0);
        fs::set_permissions(&mounted, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(fs::read(mounted.join("external")).unwrap(), b"mounted data");
        assert!(!l.home.join("files/removable").exists());
    }
    #[test]
    fn mounted_regular_child_is_detected_before_deleting_siblings() {
        let t = Temp::new();
        let l = t.layout();
        let tree = l.home.join("files/tree");
        fs::create_dir_all(&tree).unwrap();
        fs::create_dir(l.home.join("info")).unwrap();
        fs::write(tree.join("keep"), b"keep sibling").unwrap();
        let mounted = tree.join("mounted-file");
        fs::write(&mounted, b"mounted data").unwrap();
        let mounted_inode = fs::metadata(&mounted).unwrap().ino();
        let error = empty_with_mounts(&l, &t.ctx(), &|file| {
            let real = mount_id(file)?;
            Ok(real + u64::from(file.metadata()?.ino() == mounted_inode))
        })
        .unwrap_err();
        assert_eq!(error.code, "partial");
        assert_eq!(error.data.unwrap()["removed"], 0);
        assert_eq!(fs::read(tree.join("keep")).unwrap(), b"keep sibling");
        assert_eq!(fs::read(&mounted).unwrap(), b"mounted data");
        assert_eq!(empty(&l, &t.ctx()).unwrap()["removed"], 1);
    }
    #[test]
    fn separately_mounted_info_is_preserved_and_blocks_permanent_empty() {
        let t = Temp::new();
        let l = t.layout();
        fs::create_dir_all(l.home.join("files")).unwrap();
        fs::create_dir(l.home.join("info")).unwrap();
        fs::write(l.home.join("files/keep"), b"keep payload").unwrap();
        fs::write(l.home.join("info/keep.trashinfo"), b"keep metadata").unwrap();
        fs::write(l.home.join("info/orphan.trashinfo"), b"keep orphan").unwrap();
        let info_inode = fs::metadata(l.home.join("info")).unwrap().ino();
        let error = empty_with_mounts(&l, &t.ctx(), &|file| {
            let real = mount_id(file)?;
            Ok(real + u64::from(file.metadata()?.ino() == info_inode))
        })
        .unwrap_err();
        assert_eq!(error.code, "partial");
        // the test context is English (D-G1a: one item is singular now)
        assert_eq!(error.message, "1 item couldn’t be deleted.");
        assert_eq!(error.json()["key"], "files.emptyTrashFailed");
        assert_eq!(error.json()["params"], json!({"n":"1"}));
        assert_eq!(error.data.unwrap()["removed"], 0);
        assert_eq!(
            fs::read(l.home.join("files/keep")).unwrap(),
            b"keep payload"
        );
        assert_eq!(
            fs::read(l.home.join("info/keep.trashinfo")).unwrap(),
            b"keep metadata"
        );
        assert_eq!(
            fs::read(l.home.join("info/orphan.trashinfo")).unwrap(),
            b"keep orphan"
        );
        assert_eq!(empty(&l, &t.ctx()).unwrap()["removed"], 1);
    }
    #[test]
    fn readonly_folders_orphan_info_and_expunged_entries_are_removed() {
        let t = Temp::new();
        let l = t.layout();
        let c = t.ctx();
        fs::create_dir_all(l.home.join("files/folder/sub")).unwrap();
        fs::write(l.home.join("files/folder/sub/file"), b"x").unwrap();
        fs::create_dir(l.home.join("info")).unwrap();
        fs::write(l.home.join("info/orphan.trashinfo"), b"orphan").unwrap();
        fs::create_dir(l.home.join("expunged")).unwrap();
        fs::write(l.home.join("expunged/leftover"), b"x").unwrap();
        fs::set_permissions(
            l.home.join("files/folder/sub"),
            fs::Permissions::from_mode(0),
        )
        .unwrap();
        fs::set_permissions(
            l.home.join("files/folder"),
            fs::Permissions::from_mode(0o500),
        )
        .unwrap();
        assert_eq!(empty(&l, &c).unwrap()["removed"], 1);
        assert!(!l.home.join("info/orphan.trashinfo").exists());
        assert!(!l.home.join("expunged/leftover").exists());
    }
    #[test]
    fn topdir_original_paths_cannot_escape_drive_or_follow_parent_alias() {
        let t = Temp::new();
        let top = t.root.join("drive");
        let outside = t.root.join("outside");
        fs::create_dir(&top).unwrap();
        fs::create_dir(&outside).unwrap();
        assert!(original_path("../../outside", Some(&top)).is_none());
        assert!(original_path(outside.to_str().unwrap(), Some(&top)).is_none());
        assert!(original_path(".", Some(&top)).is_none());
        assert_eq!(
            original_path("folder/a%20b", Some(&top)),
            Some(top.join("folder/a%20b"))
        );
        symlink(&outside, top.join("alias")).unwrap();
        assert!(!inside(&top.join("alias/new"), &top));
    }
    #[test]
    fn mounted_trash_requires_own_real_dirs_and_sticky_shared_trash() {
        let t = Temp::new();
        let mount = t.root.join("drive");
        let uid = unsafe { libc::getuid() };
        let shared = mount.join(".Trash");
        let folder = shared.join(uid.to_string());
        fs::create_dir_all(folder.join("files")).unwrap();
        let mut l = t.layout();
        l.manual = false;
        l.mounts = vec![mount.clone()];
        assert!(l.dirs().is_empty());
        fs::set_permissions(&shared, fs::Permissions::from_mode(0o1777)).unwrap();
        assert_eq!(l.dirs().len(), 1);
        fs::remove_dir(folder.join("files")).unwrap();
        symlink(t.root.join("outside"), folder.join("files")).unwrap();
        assert!(l.dirs().is_empty());
    }
    #[test]
    fn malformed_info_and_missing_restore_have_original_errors() {
        let t = Temp::new();
        let l = t.layout();
        let c = t.ctx();
        fs::create_dir_all(l.home.join("files")).unwrap();
        fs::create_dir(l.home.join("info")).unwrap();
        fs::write(l.home.join("files/unknown"), b"x").unwrap();
        fs::write(
            l.home.join("info/unknown.trashinfo"),
            b"[Other]\nPath=/tmp/no\n",
        )
        .unwrap();
        let result = restore(&l, &json!({"names":["unknown","missing"]}), &c).unwrap();
        assert_eq!(result["ok"], false);
        assert_eq!(result["failed"].as_array().unwrap().len(), 2);
        assert_eq!(result["failed"][0]["name"], "unknown");
        assert_eq!(result["failed"][1]["name"], "missing");
        // failed[].error is rendered in the context's language (en here)
        assert_eq!(
            result["failed"][0]["error"],
            "The original location is unknown."
        );
        assert_eq!(
            result["failed"][1]["error"],
            "This item is no longer in the Recycle Bin."
        );
        let none = restore(&l, &json!({}), &c).unwrap_err();
        assert_eq!(
            (none.code.as_str(), none.message.as_str()),
            ("bad_args", "Select one or more items first.")
        );
    }
    #[test]
    fn orphan_info_collision_is_not_overwritten_and_empty_removes_it() {
        let t = Temp::new();
        let l = t.layout();
        let c = t.ctx();
        fs::create_dir_all(l.home.join("files")).unwrap();
        fs::create_dir(l.home.join("info")).unwrap();
        fs::write(l.home.join("info/a.tar.gz.trashinfo"), b"orphan-sentinel").unwrap();
        let source = t.root.join("a.tar.gz");
        fs::write(&source, b"archive").unwrap();
        let result = trash(&l, &json!({"paths":[source]}), &c).unwrap();
        assert_eq!(result["trashed"][0]["name"], "a (2).tar.gz");
        assert_eq!(
            fs::read(l.home.join("info/a.tar.gz.trashinfo")).unwrap(),
            b"orphan-sentinel"
        );
        assert_eq!(empty(&l, &c).unwrap()["removed"], 1);
        assert!(!l.home.join("info/a.tar.gz.trashinfo").exists());
    }
    #[test]
    fn restore_creates_missing_parents_and_preserves_existing_files() {
        let t = Temp::new();
        let l = t.layout();
        let c = t.ctx();
        let parent = t.root.join("parent/sub");
        fs::create_dir_all(&parent).unwrap();
        let source = parent.join("item.txt");
        fs::write(&source, b"content").unwrap();
        trash(&l, &json!({"paths":[source]}), &c).unwrap();
        fs::remove_dir_all(t.root.join("parent")).unwrap();
        let restored = restore(&l, &json!({"all":true}), &c).unwrap();
        assert_eq!(restored["ok"], true);
        assert_eq!(restored["restored"][0], json!(source));
        assert_eq!(fs::read(&source).unwrap(), b"content");
    }
    #[test]
    fn cache_detects_same_trash_name_replaced_by_another_host() {
        let t = Temp::new();
        let l = t.layout();
        fs::create_dir_all(l.home.join("files")).unwrap();
        let path = l.home.join("files/shared-name");
        fs::write(&path, b"x").unwrap();
        assert_eq!(info(&l).unwrap()["size"], 1);
        fs::remove_file(&path).unwrap();
        fs::write(&path, b"new content").unwrap();
        assert_eq!(info(&l).unwrap()["size"], 11);
    }
}
