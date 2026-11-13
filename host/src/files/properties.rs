//! Explorer Properties and attribute changes, keeping the reference JSON and counting semantics.
use super::{ApiError, ApiResult, Context, abs_in, io_error, paths};
use crate::i18n::Lang;
use gio::prelude::*;
use gtk::gio;
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, HashSet},
    ffi::{CStr, CString},
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt},
        },
    },
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(super) fn call(method: &str, args: &Value, ctx: &Context) -> Option<ApiResult> {
    match method {
        "files.properties" => Some(properties(args, ctx)),
        "files.setAttributes" => Some(set_attributes(args, ctx)),
        _ => None,
    }
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    }
}

fn budget(value: &Value) -> Duration {
    let seconds = match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        Value::Bool(value) => Some(if *value { 1.0 } else { 0.0 }),
        _ => None,
    }
    .unwrap_or(3.0);
    // Python min(10.0, NaN) retains its first operand, then max(0.2, ...) is 10.
    Duration::from_secs_f64(if seconds.is_nan() {
        10.0
    } else {
        seconds.clamp(0.2, 10.0)
    })
}

fn milliseconds(seconds: i64, nanoseconds: i64) -> i64 {
    // Match Python's float st_mtime/st_atime conversion, including truncation before the epoch.
    ((seconds as f64 + nanoseconds as f64 / 1_000_000_000.0) * 1000.0) as i64
}

fn properties(args: &Value, ctx: &Context) -> ApiResult {
    let lang = ctx.lang();
    let budget = budget(args.get("budget").unwrap_or(&json!(3.0)));
    if truthy(&args["paths"]) && !truthy(&args["path"]) {
        return multi_properties(paths(&args["paths"], lang)?, budget, ctx);
    }
    let path = abs_in(&args["path"], lang)?;
    let link = fs::symlink_metadata(&path).map_err(|error| io_error(error, &path, lang))?;
    let item = ctx.item(&path)?;
    let metadata = fs::metadata(&path).unwrap_or_else(|_| link.clone());
    let is_dir = item["isDir"] == true;
    let (size, contains, partial, on_disk) = if is_dir {
        let (size, files, folders, complete) = du(&path, Instant::now() + budget);
        (
            size,
            json!({"files":files,"folders":folders}),
            !complete,
            Value::Null,
        )
    } else {
        (
            item["size"].as_u64().unwrap_or(0),
            Value::Null,
            false,
            json!(metadata.blocks().saturating_mul(512)),
        )
    };
    let created = gio::File::for_path(&path)
        .query_info(
            "time::created",
            gio::FileQueryInfoFlags::NONE,
            gio::Cancellable::NONE,
        )
        .ok()
        .filter(|info| info.has_attribute("time::created"))
        .map(|info| info.attribute_uint64("time::created").saturating_mul(1000));
    let target = if item["isSymlink"] == true {
        Some(
            fs::read_link(&path)
                .map_err(|error| io_error(error, &path, lang))?
                .to_string_lossy()
                .into_owned(),
        )
    } else {
        None
    };
    Ok(json!({
        "name":item["name"],"displayName":item["displayName"],"path":path,
        "location":parent(&path),"type":item["typeName"],"mime":item["mime"],"icon":item["icon"],
        "isDir":item["isDir"],"isDesktopEntry":item["isDesktopEntry"],"size":size,
        "sizeOnDisk":on_disk,"contains":contains,"partial":partial,"created":created,
        "modified":milliseconds(metadata.mtime(),metadata.mtime_nsec()),
        "accessed":milliseconds(metadata.atime(),metadata.atime_nsec()),
        "owner":owner_name(metadata.uid()),"group":group_name(metadata.gid()),
        "permissions":permissions(metadata.mode()),"mode":format!("0o{:o}",metadata.mode() & 0o7777),
        "readOnly":!writable(&path),"hidden":item["hidden"],"isSymlink":item["isSymlink"],"target":target,
        "executable":metadata.mode() & 0o100 != 0 && !is_dir,
    }))
}

fn multi_properties(paths: Vec<PathBuf>, budget: Duration, ctx: &Context) -> ApiResult {
    let lang = ctx.lang();
    let deadline = Instant::now() + budget;
    let (mut size, mut files, mut folders, mut complete) = (0_u64, 0_u64, 0_u64, true);
    let mut mimes = HashSet::new();
    for path in &paths {
        let Ok(metadata) = fs::symlink_metadata(path) else {
            continue;
        };
        if metadata.is_dir() {
            folders += 1;
            mimes.insert("inode/directory".to_owned());
            let (bytes, nf, nd, finished) = du(path, deadline);
            size = size.saturating_add(bytes);
            files += nf;
            folders += nd;
            complete &= finished;
        } else {
            files += 1;
            size = size.saturating_add(metadata.len());
            mimes.insert(super::read::filename_mime(
                path.file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy()
                    .as_ref(),
            ));
        }
    }
    let kind = if mimes.len() == 1 {
        let mime = mimes.iter().next().unwrap();
        let name = paths[0]
            .file_name()
            .unwrap_or(paths[0].as_os_str())
            .to_string_lossy();
        // Two pieces, as 3b898e6 built it: "Alle van het type " + the type name.
        format!(
            "{}{}",
            crate::i18n::tr(lang, "files.allOfTypePrefix"),
            super::read::type_name(mime, &name, &ctx.language)
        )
    } else {
        crate::i18n::tr(lang, "files.multipleTypes")
    };
    let parents: HashSet<_> = paths
        .iter()
        .map(|path| parent(path).to_path_buf())
        .collect();
    let location = if parents.len() == 1 {
        parents.into_iter().next()
    } else {
        None
    };
    Ok(
        json!({"name":format!("{} items",paths.len()),"path":null,"type":kind,"size":size,
        "contains":{"files":files,"folders":folders},"partial":!complete,"location":location,"count":paths.len()}),
    )
}

/// Reference counts root-directory children but not the root itself; links count as files.
/// Directory snapshots prevent a changed child path from redirecting traversal through a link.
pub(super) fn du(path: &Path, deadline: Instant) -> (u64, u64, u64, bool) {
    let Ok(root) = fs::symlink_metadata(path) else {
        return (0, 0, 0, true);
    };
    if !root.is_dir() {
        return (root.len(), 1, 0, true);
    }
    let device = root.dev();
    let mut stack = vec![(path.to_path_buf(), root)];
    let (mut size, mut files, mut folders, mut count) = (0_u64, 0_u64, 0_u64, 0_u64);
    while let Some((path, expected)) = stack.pop() {
        let Ok(directory) = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
        else {
            continue;
        };
        let Ok(opened) = directory.metadata() else {
            continue;
        };
        if !same_inode(&opened, &expected) {
            continue;
        }
        // /proc resolves this held descriptor, rather than re-resolving a replaceable path.
        let Ok(entries) = fs::read_dir(format!("/proc/self/fd/{}", directory.as_raw_fd())) else {
            continue;
        };
        for entry in entries.flatten() {
            count += 1;
            if count % 512 == 0 && Instant::now() > deadline {
                return (size, files, folders, false);
            }
            let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if metadata.is_dir() {
                folders += 1;
                if metadata.dev() == device {
                    stack.push((path.join(entry.file_name()), metadata));
                }
            } else {
                files += 1;
                size = size.saturating_add(metadata.len());
            }
        }
        if Instant::now() > deadline && !stack.is_empty() {
            return (size, files, folders, false);
        }
    }
    (size, files, folders, true)
}

fn parent(path: &Path) -> &Path {
    path.parent().unwrap_or(Path::new("/"))
}
fn same_inode(first: &Metadata, second: &Metadata) -> bool {
    first.dev() == second.dev()
        && first.ino() == second.ino()
        && first.mode() & libc::S_IFMT == second.mode() & libc::S_IFMT
}
fn writable(path: &Path) -> bool {
    let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    unsafe { libc::access(path.as_ptr(), libc::W_OK) == 0 }
}

fn owner_name(id: u32) -> String {
    let mut bytes = vec![0_u8; 16384];
    loop {
        let mut value = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut found = std::ptr::null_mut();
        let result = unsafe {
            libc::getpwuid_r(
                id,
                value.as_mut_ptr(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
                &mut found,
            )
        };
        if result == libc::ERANGE && bytes.len() < 1024 * 1024 {
            bytes.resize(bytes.len() * 2, 0);
            continue;
        }
        if result != 0 || found.is_null() {
            return id.to_string();
        }
        return unsafe { CStr::from_ptr(value.assume_init().pw_name) }
            .to_string_lossy()
            .into_owned();
    }
}
fn group_name(id: u32) -> String {
    let mut bytes = vec![0_u8; 16384];
    loop {
        let mut value = std::mem::MaybeUninit::<libc::group>::uninit();
        let mut found = std::ptr::null_mut();
        let result = unsafe {
            libc::getgrgid_r(
                id,
                value.as_mut_ptr(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
                &mut found,
            )
        };
        if result == libc::ERANGE && bytes.len() < 1024 * 1024 {
            bytes.resize(bytes.len() * 2, 0);
            continue;
        }
        if result != 0 || found.is_null() {
            return id.to_string();
        }
        return unsafe { CStr::from_ptr(value.assume_init().gr_name) }
            .to_string_lossy()
            .into_owned();
    }
}
fn permissions(mode: u32) -> String {
    let mut output = [b'-'; 9];
    for (index, bit, symbol) in [
        (0, 0o400, b'r'),
        (1, 0o200, b'w'),
        (2, 0o100, b'x'),
        (3, 0o040, b'r'),
        (4, 0o020, b'w'),
        (5, 0o010, b'x'),
        (6, 0o004, b'r'),
        (7, 0o002, b'w'),
        (8, 0o001, b'x'),
    ] {
        if mode & bit != 0 {
            output[index] = symbol;
        }
    }
    for (index, bit, lower, upper) in [
        (2, 0o4000, b's', b'S'),
        (5, 0o2000, b's', b'S'),
        (8, 0o1000, b't', b'T'),
    ] {
        if mode & bit != 0 {
            output[index] = if output[index] == b'x' { lower } else { upper };
        }
    }
    String::from_utf8(output.to_vec()).unwrap()
}

fn set_attributes(args: &Value, ctx: &Context) -> ApiResult {
    let lang = ctx.lang();
    let path = abs_in(&args["path"], lang)?;
    let metadata = fs::symlink_metadata(&path)
        .map_err(|_| ApiError::tr("not_found", lang, "files.itemGone", crate::i18n::NO_PARAMS))?;
    if let Some(value) = args.get("readOnly").filter(|value| !value.is_null()) {
        if metadata.file_type().is_symlink() {
            return Err(ApiError::tr(
                "symlink",
                lang,
                "files.symlink",
                crate::i18n::NO_PARAMS,
            ));
        }
        set_read_only(&path, &metadata, truthy(value), lang)?;
    }
    if let Some(value) = args.get("hidden").filter(|value| !value.is_null()) {
        let name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy();
        if name.starts_with('.') {
            if !truthy(value) {
                return Err(ApiError::tr(
                    "dotfile",
                    lang,
                    "files.dotfile",
                    crate::i18n::NO_PARAMS,
                ));
            }
        } else {
            set_hidden(parent(&path), &name, truthy(value), lang)?;
        }
    }
    ctx.changed(parent(&path));
    Ok(json!({"ok":true}))
}

fn set_read_only(
    path: &Path,
    expected: &Metadata,
    readonly: bool,
    lang: Lang,
) -> Result<(), ApiError> {
    let descriptor = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| io_error(error, path, lang))?;
    let metadata = descriptor
        .metadata()
        .map_err(|error| io_error(error, path, lang))?;
    if metadata.file_type().is_symlink() {
        return Err(ApiError::tr(
            "symlink",
            lang,
            "files.symlink",
            crate::i18n::NO_PARAMS,
        ));
    }
    if !same_inode(&metadata, expected) {
        return Err(ApiError::tr(
            "busy",
            lang,
            "files.changedMeanwhile",
            crate::i18n::NO_PARAMS,
        ));
    }
    let mode = metadata.mode() & 0o7777;
    let mode = if readonly {
        mode & !0o222
    } else {
        mode | 0o200
    };
    // An O_PATH descriptor does not require read permission and never follows a changed link.
    // chmod through its proc descriptor names the held inode, not the original filesystem path.
    let descriptor_path =
        CString::new(format!("/proc/self/fd/{}", descriptor.as_raw_fd())).unwrap();
    if unsafe { libc::chmod(descriptor_path.as_ptr(), mode) } == -1 {
        return Err(io_error(io::Error::last_os_error(), path, lang));
    }
    Ok(())
}

fn set_hidden(directory: &Path, name: &str, hidden: bool, lang: Lang) -> Result<(), ApiError> {
    let descriptor = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open(directory)
        .or_else(|error| {
            if error.kind() == io::ErrorKind::PermissionDenied {
                OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC)
                    .open(directory)
            } else {
                Err(error)
            }
        })
        .map_err(|error| io_error(error, &directory.join(".hidden"), lang))?;
    // Directory flock serializes our own writers without creating an extra visible lock file.
    let _lock = DirectoryLock::new(&descriptor);
    let hidden_name = CString::new(".hidden").unwrap();
    let mut names = hidden_names_at(&descriptor);
    names.remove(name);
    if hidden {
        names.insert(name.into());
    }
    if names.is_empty() {
        let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
        // Match os.path.exists: a dangling .hidden link is left alone rather than removed.
        if unsafe {
            libc::fstatat(
                descriptor.as_raw_fd(),
                hidden_name.as_ptr(),
                metadata.as_mut_ptr(),
                0,
            )
        } == -1
        {
            return Ok(());
        }
        if unsafe { libc::unlinkat(descriptor.as_raw_fd(), hidden_name.as_ptr(), 0) } == -1 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::NotFound {
                return Err(io_error(error, &directory.join(".hidden"), lang));
            }
        }
        return Ok(());
    }
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = CString::new(format!(
        ".hidden.{}.{}.{}.tmp",
        std::process::id(),
        stamp,
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
    .unwrap();
    let fd = unsafe {
        libc::openat(
            descriptor.as_raw_fd(),
            temporary.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o644,
        )
    };
    if fd == -1 {
        return Err(io_error(
            io::Error::last_os_error(),
            &directory.join(".hidden"),
            lang,
        ));
    }
    let result = (|| {
        let mut file = unsafe { File::from_raw_fd(fd) };
        let text = names.into_iter().collect::<Vec<_>>().join("\n") + "\n";
        file.write_all(text.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|error| io_error(error, &directory.join(".hidden"), lang))?;
        if unsafe { libc::fchmod(file.as_raw_fd(), 0o644) } == -1 {
            return Err(io_error(
                io::Error::last_os_error(),
                &directory.join(".hidden"),
                lang,
            ));
        }
        if unsafe {
            libc::renameat(
                descriptor.as_raw_fd(),
                temporary.as_ptr(),
                descriptor.as_raw_fd(),
                hidden_name.as_ptr(),
            )
        } == -1
        {
            return Err(io_error(
                io::Error::last_os_error(),
                &directory.join(".hidden"),
                lang,
            ));
        }
        Ok(())
    })();
    if result.is_err() {
        unsafe {
            libc::unlinkat(descriptor.as_raw_fd(), temporary.as_ptr(), 0);
        }
    }
    result
}

fn hidden_names_at(directory: &File) -> BTreeSet<String> {
    let name = CString::new(".hidden").unwrap();
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd == -1 {
        return BTreeSet::new();
    }
    let file = unsafe { File::from_raw_fd(fd) };
    if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        return BTreeSet::new();
    }
    let mut bytes = Vec::new();
    if file.take(1024 * 1024).read_to_end(&mut bytes).is_err() {
        return BTreeSet::new();
    }
    String::from_utf8_lossy(&bytes)
        .chars()
        .take(256 * 1024)
        .collect::<String>()
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}
struct DirectoryLock(Option<i32>);
impl DirectoryLock {
    fn new(file: &File) -> Self {
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
                return Self(Some(file.as_raw_fd()));
            }
            if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                return Self(None);
            }
        }
    }
}
impl Drop for DirectoryLock {
    fn drop(&mut self) {
        if let Some(fd) = self.0 {
            unsafe {
                libc::flock(fd, libc::LOCK_UN);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("win11-properties-{}", crate::events::nonce()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn ctx(&self) -> Context {
            Context::new(self.0.clone(), self.0.join("Desktop"), "en".into(), false)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn file_properties_keep_reference_metadata_and_special_mode_bits() {
        let fixture = Fixture::new();
        let path = fixture.0.join("note.txt");
        fs::write(&path, b"hello").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o4751)).unwrap();
        let value = properties(&json!({"path":path}), &fixture.ctx()).unwrap();
        assert_eq!(value["size"], 5);
        assert_eq!(value["name"], "note.txt");
        assert_eq!(value["mime"], "text/plain");
        assert_eq!(value["type"], "Text Document");
        assert_eq!(value["mode"], "0o4751");
        assert_eq!(value["permissions"], "rwsr-x--x");
        assert_eq!(value["contains"], Value::Null);
        assert_eq!(value["executable"], true);
        assert_eq!(value["target"], Value::Null);
        assert!(value["owner"].as_str().is_some_and(|name| !name.is_empty()));
        assert!(value["group"].as_str().is_some_and(|name| !name.is_empty()));
    }
    #[test]
    fn directory_counting_counts_links_without_following_cycles_or_external_trees() {
        let fixture = Fixture::new();
        let root = fixture.0.join("folder");
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("sub")).unwrap();
        fs::write(root.join("a"), b"123").unwrap();
        fs::write(root.join("sub/b"), b"45").unwrap();
        symlink(".", root.join("cycle")).unwrap();
        symlink("../missing", root.join("broken")).unwrap();
        let expected = 5
            + fs::symlink_metadata(root.join("cycle")).unwrap().len()
            + fs::symlink_metadata(root.join("broken")).unwrap().len();
        assert_eq!(
            du(&root, Instant::now() + Duration::from_secs(2)),
            (expected, 4, 1, true)
        );
        let value = properties(&json!({"path":root}), &fixture.ctx()).unwrap();
        assert_eq!(value["contains"], json!({"files":4,"folders":1}));
        assert_eq!(value["sizeOnDisk"], Value::Null);
    }
    #[test]
    fn link_properties_follow_display_metadata_but_directory_count_keeps_reference_link_semantics()
    {
        let fixture = Fixture::new();
        let folder = fixture.0.join("folder");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("a"), b"abc").unwrap();
        let path = fixture.0.join("alias");
        symlink("folder", &path).unwrap();
        let value = properties(&json!({"path":path}), &fixture.ctx()).unwrap();
        assert_eq!(value["isDir"], true);
        assert_eq!(value["isSymlink"], true);
        assert_eq!(value["target"], "folder");
        assert_eq!(value["size"], 6);
        assert_eq!(value["contains"], json!({"files":1,"folders":0}));
    }
    #[test]
    fn multi_selection_uses_filename_only_mime_and_includes_selected_root_folders() {
        let fixture = Fixture::new();
        let a = fixture.0.join("a.txt");
        let b = fixture.0.join("b.txt");
        fs::write(&a, b"a").unwrap();
        fs::write(&b, b"bb").unwrap();
        let value = properties(&json!({"paths":[a,b]}), &fixture.ctx()).unwrap();
        assert_eq!(value["type"], "All of type Text Document");
        assert_eq!(value["size"], 3);
        assert_eq!(value["contains"], json!({"files":2,"folders":0}));
        let directory = fixture.0.join("folder");
        fs::create_dir(&directory).unwrap();
        let value = properties(&json!({"paths":[directory]}), &fixture.ctx()).unwrap();
        assert_eq!(value["type"], "All of type File folder");
        assert_eq!(value["contains"], json!({"files":0,"folders":1}));
    }
    #[test]
    fn hidden_and_read_only_attributes_preserve_other_names_and_restore_owner_write() {
        let fixture = Fixture::new();
        let path = fixture.0.join("note.txt");
        fs::write(&path, b"hello").unwrap();
        fs::write(fixture.0.join(".hidden"), "other.txt\n").unwrap();
        set_attributes(
            &json!({"path":path,"hidden":true,"readOnly":true}),
            &fixture.ctx(),
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(fixture.0.join(".hidden")).unwrap(),
            "note.txt\nother.txt\n"
        );
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o222, 0);
        assert_eq!(
            properties(&json!({"path":path}), &fixture.ctx()).unwrap()["hidden"],
            true
        );
        set_attributes(
            &json!({"path":path,"hidden":false,"readOnly":false}),
            &fixture.ctx(),
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(fixture.0.join(".hidden")).unwrap(),
            "other.txt\n"
        );
        assert_ne!(fs::metadata(path).unwrap().mode() & 0o200, 0);
    }
    #[test]
    fn read_only_never_changes_a_symlink_target_or_replaced_inode() {
        let fixture = Fixture::new();
        let target = fixture.0.join("target");
        fs::write(&target, b"keep").unwrap();
        let link = fixture.0.join("link");
        symlink("target", &link).unwrap();
        let mode = fs::metadata(&target).unwrap().mode();
        assert_eq!(
            set_attributes(&json!({"path":link,"readOnly":true}), &fixture.ctx())
                .unwrap_err()
                .code,
            "symlink"
        );
        assert_eq!(fs::metadata(&target).unwrap().mode(), mode);
        let expected = fs::metadata(&target).unwrap();
        fs::rename(&target, fixture.0.join("original")).unwrap();
        symlink("original", &target).unwrap();
        assert_eq!(
            set_read_only(&target, &expected, true, Lang::Nl)
                .unwrap_err()
                .code,
            "symlink"
        );
        assert_eq!(
            fs::metadata(fixture.0.join("original")).unwrap().mode(),
            mode
        );
    }
    #[test]
    fn hidden_updates_replace_link_metadata_without_writing_to_its_external_target() {
        let fixture = Fixture::new();
        let target = fixture.0.join("external");
        fs::write(&target, "kept.txt\n").unwrap();
        let directory = fixture.0.join("folder");
        fs::create_dir(&directory).unwrap();
        let path = directory.join("note.txt");
        fs::write(&path, b"note").unwrap();
        symlink("../external", directory.join(".hidden")).unwrap();
        set_attributes(&json!({"path":path,"hidden":true}), &fixture.ctx()).unwrap();
        assert_eq!(fs::read_to_string(target).unwrap(), "kept.txt\n");
        assert!(
            !fs::symlink_metadata(directory.join(".hidden"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_to_string(directory.join(".hidden")).unwrap(),
            "kept.txt\nnote.txt\n"
        );
    }
    #[test]
    fn concurrent_hidden_updates_keep_every_native_selection() {
        let fixture = Fixture::new();
        let mut threads = Vec::new();
        for index in 0..8 {
            let root = fixture.0.clone();
            let path = root.join(format!("item{index}"));
            fs::write(&path, b"x").unwrap();
            threads.push(std::thread::spawn(move || {
                let ctx = Context::new(root.clone(), root.join("Desktop"), "en".into(), false);
                set_attributes(&json!({"path":path,"hidden":true}), &ctx).unwrap();
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }
        let value = fs::read_to_string(fixture.0.join(".hidden")).unwrap();
        assert_eq!(
            value.lines().collect::<BTreeSet<_>>(),
            (0..8)
                .map(|index| format!("item{index}"))
                .collect::<BTreeSet<_>>()
                .iter()
                .map(String::as_str)
                .collect()
        );
    }
    #[test]
    fn owner_write_can_be_restored_even_when_the_file_has_no_read_permission() {
        let fixture = Fixture::new();
        let path = fixture.0.join("private.txt");
        fs::write(&path, b"x").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0)).unwrap();
        set_attributes(&json!({"path":path,"readOnly":false}), &fixture.ctx()).unwrap();
        assert_eq!(fs::metadata(path).unwrap().mode() & 0o777, 0o200);
    }
    #[test]
    fn dotfiles_missing_paths_and_budgets_keep_reference_errors_and_bounds() {
        let fixture = Fixture::new();
        let dot = fixture.0.join(".dot");
        fs::write(&dot, b"x").unwrap();
        assert_eq!(
            set_attributes(&json!({"path":dot,"hidden":false}), &fixture.ctx())
                .unwrap_err()
                .code,
            "dotfile"
        );
        assert_eq!(
            set_attributes(
                &json!({"path":fixture.0.join("missing"),"hidden":true}),
                &fixture.ctx()
            )
            .unwrap_err()
            .code,
            "not_found"
        );
        assert_eq!(budget(&json!(false)), Duration::from_millis(200));
        assert_eq!(budget(&json!("bad")), Duration::from_secs(3));
        assert_eq!(budget(&json!("NaN")), Duration::from_secs(10));
        assert_eq!(budget(&json!(200)), Duration::from_secs(10));
        assert_eq!(milliseconds(-1, 999_999_999), 0);
        assert_eq!(permissions(0o7644), "rwSr-Sr-T");
    }
}
