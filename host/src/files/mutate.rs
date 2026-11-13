//! Conflict-safe Explorer writes. Existing names, including dangling links, are never replaced.
use super::{ApiError, ApiResult, Context, abs_in, directory, io_error, paths, validate_name};
use serde_json::{Value, json};
use std::{
    ffi::{CString, OsStr, OsString},
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            ffi::OsStrExt,
            fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, symlink},
        },
    },
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const COMPOUND_EXTENSIONS: [&str; 6] = [
    ".tar.gz",
    ".tar.xz",
    ".tar.bz2",
    ".tar.zst",
    ".tar.lz",
    ".tar.lzma",
];
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) fn call(method: &str, args: &Value, ctx: &Context) -> Option<ApiResult> {
    match method {
        "files.createFolder" => Some(create(args, ctx, true)),
        "files.createFile" => Some(create(args, ctx, false)),
        "files.rename" => Some(rename(args, ctx)),
        "files.copy" | "files.move" => Some((|| {
            let sources = paths(&args["paths"], ctx.lang())?;
            let dest = directory(&args["dest"], ctx.lang())?;
            let job = args.get("jobId").and_then(Value::as_str);
            transfer(sources, dest, method == "files.move", job, ctx)
        })()),
        _ => None,
    }
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

fn create(args: &Value, ctx: &Context, folder: bool) -> ApiResult {
    let lang = ctx.lang();
    let dest = directory(&args["dir"], lang)?;
    let kind = if !truthy(&args["kind"]) {
        "text"
    } else {
        args["kind"].as_str().unwrap_or("")
    };
    if !folder && !matches!(kind, "text" | "empty") {
        return Err(ApiError::tr(
            "bad_args",
            lang,
            "files.unknownKind",
            crate::i18n::NO_PARAMS,
        ));
    }
    let name = if truthy(&args["name"]) {
        validate_name(&args["name"], lang)?
    } else {
        match (folder, kind) {
            (true, _) => crate::i18n::tr(lang, "files.new.folder"),
            (_, "empty") => crate::i18n::tr(lang, "files.new.empty"),
            _ => crate::i18n::tr(lang, "files.new.text"),
        }
    };
    for _ in 0..20 {
        let target =
            unique_path(&dest, &name, folder).map_err(|error| io_error(error, &dest, lang))?;
        let result = if folder {
            fs::DirBuilder::new().mode(0o755).create(&target)
        } else {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o644)
                .open(&target)
                .map(drop)
        };
        match result {
            Ok(()) => {
                ctx.changed(&dest);
                return Ok(json!({"path":target,"item":ctx.item(&target)?}));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(io_error(error, &target, lang)),
        }
    }
    Err(ApiError::tr(
        "exists",
        lang,
        "files.exists",
        crate::i18n::NO_PARAMS,
    ))
}

fn rename(args: &Value, ctx: &Context) -> ApiResult {
    let lang = ctx.lang();
    let source = abs_in(&args["path"], lang)?;
    let name = validate_name(&args["newName"], lang)?;
    if let Err(error) = fs::symlink_metadata(&source) {
        return if error.kind() == io::ErrorKind::NotFound {
            Err(ApiError::tr(
                "not_found",
                lang,
                "files.itemGone",
                crate::i18n::NO_PARAMS,
            ))
        } else {
            Err(io_error(error, &source, lang))
        };
    }
    let parent = source.parent().unwrap_or(Path::new("/"));
    let target = parent.join(&name);
    if target != source {
        let conflict = || {
            ApiError::tr(
                "exists",
                lang,
                "files.nameExists",
                &serde_json::json!({"name": name.to_string()}),
            )
        };
        if lexists(&target) {
            if !same_file(&source, &target) {
                return Err(conflict());
            }
            // A case-only rename on case-insensitive media needs an intermediate name.
            // On case-sensitive media two different hardlink directory entries must also
            // be retained; do not erase one of those names to imitate a case-only rename.
            if fs::read_dir(parent)
                .map(|entries| {
                    entries
                        .flatten()
                        .any(|entry| entry.file_name() == OsStr::new(&name))
                })
                .unwrap_or(true)
            {
                return Err(conflict());
            }
            let temp = reserve_rename_path(parent, &name)
                .map_err(|error| io_error(error, parent, lang))?;
            rename_noreplace(&source, &temp).map_err(|error| io_error(error, &source, lang))?;
            if let Err(error) = rename_noreplace(&temp, &target) {
                // Never replace an item that appeared under the old source name either.
                if rename_noreplace(&temp, &source).is_err() {
                    return Err(ApiError::tr(
                        "failed",
                        lang,
                        "files.renameIncomplete",
                        &serde_json::json!({"path": temp.display().to_string()}),
                    ));
                }
                return Err(if error.kind() == io::ErrorKind::AlreadyExists {
                    conflict()
                } else {
                    io_error(error, &source, lang)
                });
            }
        } else if let Err(error) = rename_noreplace(&source, &target) {
            return Err(if error.kind() == io::ErrorKind::AlreadyExists {
                conflict()
            } else {
                io_error(error, &source, lang)
            });
        }
        ctx.changed(parent);
    }
    Ok(json!({"path":target,"item":ctx.item(&target)?}))
}

fn reserve_rename_path(parent: &Path, name: &str) -> io::Result<PathBuf> {
    for _ in 0..20 {
        let candidate = parent.join(format!(".{name}.{}.tmp", nonce()));
        if !lexists(&candidate) {
            return Ok(candidate);
        }
    }
    Err(io::Error::from_raw_os_error(libc::EEXIST))
}

fn nonce() -> String {
    let mut bytes = [0_u8; 12];
    if File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .is_ok()
    {
        return bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    }
    format!(
        "{:x}{:x}{:x}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

fn lexists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn same_file(a: &Path, b: &Path) -> bool {
    fs::metadata(a)
        .ok()
        .zip(fs::metadata(b).ok())
        .is_some_and(|(a, b)| a.dev() == b.dev() && a.ino() == b.ino())
}

pub(super) fn split_name(name: &str) -> (&str, &str) {
    let lower = name.to_lowercase();
    for extension in COMPOUND_EXTENSIONS {
        if lower.ends_with(extension) && name.len() > extension.len() {
            return name.split_at(name.len() - extension.len());
        }
    }
    if let Some(index) = name.rfind('.')
        && name[..index].chars().any(|character| character != '.')
    {
        return name.split_at(index);
    }
    (name, "")
}

fn numbered_stem(stem: &str) -> (&str, u64) {
    if stem.ends_with(')')
        && let Some(index) = stem.rfind(" (")
    {
        let digits = &stem[index + 2..stem.len() - 1];
        if !digits.is_empty()
            && digits.bytes().all(|byte| byte.is_ascii_digit())
            && let Ok(number) = digits.parse::<u64>()
        {
            return (&stem[..index], number.saturating_add(1));
        }
    }
    (stem, 2)
}

pub(super) fn unique_path(directory: &Path, name: &str, is_dir: bool) -> io::Result<PathBuf> {
    let first = directory.join(name);
    if !lexists(&first) {
        return Ok(first);
    }
    let (stem, extension) = if is_dir { (name, "") } else { split_name(name) };
    let (base, start) = numbered_stem(stem);
    for number in start..start.saturating_add(100_000) {
        let candidate = directory.join(format!("{base} ({number}){extension}"));
        if !lexists(&candidate) {
            return Ok(candidate);
        }
    }
    Err(io::Error::from_raw_os_error(libc::EEXIST))
}

fn copy_path(directory: &Path, name: &str, is_dir: bool, word: &str) -> io::Result<PathBuf> {
    let (stem, extension) = if is_dir { (name, "") } else { split_name(name) };
    let base = format!("{stem} - {word}");
    let first = directory.join(format!("{base}{extension}"));
    if !lexists(&first) {
        return Ok(first);
    }
    for number in 2..100_000 {
        let candidate = directory.join(format!("{base} ({number}){extension}"));
        if !lexists(&candidate) {
            return Ok(candidate);
        }
    }
    Err(io::Error::from_raw_os_error(libc::EEXIST))
}

/// There is intentionally no check-then-rename fallback: that can replace a racing destination.
pub(super) fn rename_noreplace(source: &Path, target: &Path) -> io::Result<()> {
    let source = c_path(source)?;
    let target = c_path(target)?;
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            target.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn c_path(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))
}

struct Progress<'a> {
    ctx: Option<&'a Context>,
    job: String,
    moving: bool,
    total: Option<u64>,
    done: u64,
    items: usize,
    count: usize,
    current: String,
    last: Option<Instant>,
}

impl<'a> Progress<'a> {
    fn emit(&mut self, finished: bool, force: bool) {
        if !finished
            && !force
            && self
                .last
                .is_some_and(|last| last.elapsed() < Duration::from_millis(250))
        {
            return;
        }
        self.last = Some(Instant::now());
        if let Some(ctx) = self.ctx {
            ctx.emit("files.progress", json!({"jobId":self.job,"op":if self.moving {"move"} else {"copy"},"done":self.done,"total":self.total,"items":self.items,"count":self.count,"current":self.current,"finished":finished}));
        }
    }
    fn add(&mut self, bytes: u64) {
        self.done = self.done.saturating_add(bytes);
        self.emit(false, false);
    }
}

pub(super) fn transfer(
    sources: Vec<PathBuf>,
    dest: PathBuf,
    moving: bool,
    job: Option<&str>,
    ctx: &Context,
) -> ApiResult {
    let lang = ctx.lang();
    if !dest.is_dir() {
        return Err(ApiError::tr(
            "not_found",
            lang,
            "files.destGone",
            crate::i18n::NO_PARAMS,
        ));
    }
    let mut progress = Progress {
        ctx: Some(ctx),
        job: job
            .filter(|job| !job.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(nonce),
        moving,
        total: total_size(&sources),
        done: 0,
        items: 0,
        count: sources.len(),
        current: String::new(),
        last: None,
    };
    progress.emit(false, true);
    let mut completed = Vec::new();
    let mut failed = Vec::new();
    // "<name> - kopie.txt" / "<name> - Copy.txt" for a copy into its own folder (Windows 11 style).
    let copy_word = crate::i18n::tr(lang, "files.copyWord");
    // The marker error of a folder moved into itself, and its text in failed[].
    let into_itself = crate::i18n::tr(lang, "files.intoItself");
    let dest_real = match fs::canonicalize(&dest) {
        Ok(path) => path,
        Err(error) => {
            progress.emit(true, true);
            return Err(io_error(error, &dest, lang));
        }
    };
    for source in &sources {
        let name = source.file_name().and_then(OsStr::to_str).unwrap_or("");
        progress.current = name.to_owned();
        let result = (|| {
            if name.is_empty() {
                return Err(io::Error::from_raw_os_error(libc::EINVAL));
            }
            let metadata = fs::symlink_metadata(source)?;
            if metadata.is_dir() && dest_real.starts_with(fs::canonicalize(source)?) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    into_itself.as_str(),
                ));
            }
            if moving && source.parent() == Some(dest.as_path()) {
                return Ok(source.clone());
            }
            for attempt in 0..20 {
                let target = if !moving && source.parent() == Some(dest.as_path()) {
                    copy_path(&dest, name, metadata.is_dir(), &copy_word)?
                } else {
                    unique_path(&dest, name, metadata.is_dir())?
                };
                let write = if moving {
                    match rename_noreplace(source, &target) {
                        Ok(()) => {
                            progress.add(if metadata.is_dir() { 0 } else { metadata.len() });
                            Ok(())
                        }
                        Err(error) if error.raw_os_error() == Some(libc::EXDEV) => {
                            copy_then_remove(source, &target, &mut progress)
                        }
                        Err(error) => Err(error),
                    }
                } else {
                    copy_any(source, &target, &mut progress, 0).map(|_| ())
                };
                match write {
                    Ok(()) => return Ok(target),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists && attempt < 19 => {
                        continue;
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(io::Error::from_raw_os_error(libc::EEXIST))
        })();
        match result {
            Ok(path) => {
                completed.push(path);
                if !(moving && source.parent() == Some(dest.as_path())) {
                    progress.items += 1;
                }
            }
            Err(error) => {
                let message = if error.kind() == io::ErrorKind::InvalidInput
                    && error.to_string() == into_itself
                {
                    error.to_string()
                } else {
                    io_error(error, source, lang).message
                };
                failed.push(json!({"path":source,"error":message}));
            }
        }
    }
    progress.emit(true, true);
    ctx.changed(&dest);
    if moving {
        for source in &sources {
            if let Some(parent) = source.parent() {
                ctx.changed(parent);
            }
        }
    }
    Ok(json!({"ok":failed.is_empty(),"paths":completed,"failed":failed,"jobId":progress.job}))
}

pub(super) fn total_size(paths: &[PathBuf]) -> Option<u64> {
    let deadline = Instant::now() + Duration::from_millis(1500);
    let mut total = 0_u64;
    let mut stack = paths.to_vec();
    while let Some(path) = stack.pop() {
        if Instant::now() > deadline {
            return None;
        }
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.is_dir() {
            if let Ok(entries) = fs::read_dir(path) {
                stack.extend(entries.flatten().map(|entry| entry.path()));
            }
        } else {
            total = total.saturating_add(metadata.len());
        }
    }
    Some(total)
}

#[derive(Clone, Debug)]
struct Snapshot {
    metadata: Metadata,
    children: Vec<(OsString, Snapshot)>,
}

fn same_identity(left: &Metadata, right: &Metadata) -> bool {
    left.dev() == right.dev()
        && left.ino() == right.ino()
        && (left.mode() & libc::S_IFMT) == (right.mode() & libc::S_IFMT)
}

fn unchanged(left: &Metadata, right: &Metadata) -> bool {
    same_identity(left, right)
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
        && left.ctime() == right.ctime()
        && left.ctime_nsec() == right.ctime_nsec()
}

fn copy_any(
    source: &Path,
    target: &Path,
    progress: &mut Progress<'_>,
    depth: usize,
) -> io::Result<Snapshot> {
    if depth > 256 {
        return Err(io::Error::from_raw_os_error(libc::ELOOP));
    }
    let metadata = fs::symlink_metadata(source)?;
    let mut snapshot = Snapshot {
        metadata: metadata.clone(),
        children: Vec::new(),
    };
    if metadata.file_type().is_symlink() {
        let link = fs::read_link(source)?;
        if !unchanged(&metadata, &fs::symlink_metadata(source)?) {
            return Err(io::Error::from_raw_os_error(libc::EBUSY));
        }
        symlink(link, target)?;
    } else if metadata.is_dir() {
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(source)?;
        if !same_identity(&metadata, &directory.metadata()?) {
            return Err(io::Error::from_raw_os_error(libc::EBUSY));
        }
        fs::DirBuilder::new().mode(0o700).create(target)?;
        let owned = fs::symlink_metadata(target)?;
        let result = (|| {
            let output_directory = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(target)?;
            if !same_identity(&owned, &output_directory.metadata()?) {
                return Err(io::Error::from_raw_os_error(libc::EBUSY));
            }
            let target_directory =
                PathBuf::from(format!("/proc/self/fd/{}", output_directory.as_raw_fd()));
            // Walk the open directory, so replacing a source component with a link
            // cannot redirect this recursive copy into a different tree.
            for entry in fs::read_dir(format!("/proc/self/fd/{}", directory.as_raw_fd()))? {
                let entry = entry?;
                progress.current = entry.file_name().to_string_lossy().into_owned();
                let child = copy_any(
                    &entry.path(),
                    &target_directory.join(entry.file_name()),
                    progress,
                    depth + 1,
                )?;
                snapshot.children.push((entry.file_name(), child));
            }
            preserve_metadata(&directory, &output_directory, &directory.metadata()?);
            if !same_identity(&owned, &fs::symlink_metadata(target)?) {
                return Err(io::Error::from_raw_os_error(libc::EBUSY));
            }
            Ok(())
        })();
        if let Err(error) = result {
            cleanup_owned(target, &owned);
            return Err(error);
        }
    } else if metadata.is_file() {
        let mut input = open_regular_source(source, &metadata)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(target)?;
        let owned = output.metadata()?;
        let result = (|| {
            let mut buffer = vec![0_u8; 1024 * 1024];
            loop {
                let read = input.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                output.write_all(&buffer[..read])?;
                progress.add(read as u64);
            }
            output.flush()?;
            if !unchanged(&metadata, &input.metadata()?) {
                return Err(io::Error::from_raw_os_error(libc::EBUSY));
            }
            preserve_metadata(&input, &output, &input.metadata()?);
            if !same_identity(&owned, &fs::symlink_metadata(target)?) {
                return Err(io::Error::from_raw_os_error(libc::EBUSY));
            }
            Ok(())
        })();
        if let Err(error) = result {
            drop(output);
            cleanup_owned(target, &owned);
            return Err(error);
        }
    } else {
        return Err(io::Error::from_raw_os_error(libc::EINVAL));
    }
    Ok(snapshot)
}

fn open_regular_source(path: &Path, expected: &Metadata) -> io::Result<File> {
    // O_NONBLOCK is ignored by regular files, but avoids hanging if another process
    // replaces the lstat'ed regular file with a FIFO before open. Validate before reading.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)?;
    if !same_identity(expected, &file.metadata()?) {
        return Err(io::Error::from_raw_os_error(libc::EBUSY));
    }
    Ok(file)
}

fn cleanup_owned(path: &Path, owned: &Metadata) {
    if fs::symlink_metadata(path).is_ok_and(|current| same_identity(owned, &current)) {
        if owned.is_dir() {
            let _ = fs::remove_dir_all(path);
        } else {
            let _ = fs::remove_file(path);
        }
    }
}

fn preserve_metadata(source: &File, target: &File, metadata: &Metadata) {
    let times = [
        libc::timespec {
            tv_sec: metadata.atime(),
            tv_nsec: metadata.atime_nsec(),
        },
        libc::timespec {
            tv_sec: metadata.mtime(),
            tv_nsec: metadata.mtime_nsec(),
        },
    ];
    // Operate on the exact opened source/destination inodes, including for metadata:
    // a concurrent replacement with a symlink must never chmod a different target.
    unsafe {
        libc::fchmod(target.as_raw_fd(), metadata.mode() & 0o7777);
        libc::futimens(target.as_raw_fd(), times.as_ptr());
    }
    // shutil.copystat also preserves xattrs where supported, on a best-effort basis.
    let length = unsafe { libc::flistxattr(source.as_raw_fd(), std::ptr::null_mut(), 0) };
    if length <= 0 || length > 1024 * 1024 {
        return;
    }
    let mut names = vec![0_u8; length as usize];
    let read =
        unsafe { libc::flistxattr(source.as_raw_fd(), names.as_mut_ptr().cast(), names.len()) };
    if read <= 0 {
        return;
    }
    for name in names[..read as usize]
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
    {
        let Ok(name) = CString::new(name) else {
            continue;
        };
        let length =
            unsafe { libc::fgetxattr(source.as_raw_fd(), name.as_ptr(), std::ptr::null_mut(), 0) };
        if !(0..=1024 * 1024).contains(&length) {
            continue;
        }
        let mut value = vec![0_u8; length as usize];
        let read = unsafe {
            libc::fgetxattr(
                source.as_raw_fd(),
                name.as_ptr(),
                value.as_mut_ptr().cast(),
                value.len(),
            )
        };
        if read >= 0 {
            unsafe {
                libc::fsetxattr(
                    target.as_raw_fd(),
                    name.as_ptr(),
                    value.as_ptr().cast(),
                    read as usize,
                    0,
                );
            }
        }
    }
}

fn copy_then_remove(source: &Path, target: &Path, progress: &mut Progress<'_>) -> io::Result<()> {
    let snapshot = copy_any(source, target, progress, 0)?;
    // Keep both copies rather than remove a source that changed while being copied.
    verify_snapshot(source, &snapshot)?;
    let parent = source
        .parent()
        .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?;
    let name = source
        .file_name()
        .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?;
    let parent = File::open(parent)?;
    remove_snapshot_at(&parent, name, &snapshot)
}

pub(super) fn move_noreplace_exact(source: &Path, target: &Path) -> io::Result<()> {
    match rename_noreplace(source, target) {
        Ok(()) => Ok(()),
        Err(error) if error.raw_os_error() == Some(libc::EXDEV) => {
            let mut progress = Progress {
                ctx: None,
                job: String::new(),
                moving: true,
                total: None,
                done: 0,
                items: 0,
                count: 1,
                current: String::new(),
                last: None,
            };
            copy_then_remove(source, target, &mut progress)
        }
        Err(error) => Err(error),
    }
}

fn verify_snapshot(path: &Path, snapshot: &Snapshot) -> io::Result<()> {
    if !unchanged(&snapshot.metadata, &fs::symlink_metadata(path)?) {
        return Err(io::Error::from_raw_os_error(libc::EBUSY));
    }
    if snapshot.metadata.is_dir() {
        let actual: std::collections::BTreeSet<_> = fs::read_dir(path)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<io::Result<_>>()?;
        let expected: std::collections::BTreeSet<_> = snapshot
            .children
            .iter()
            .map(|(name, _)| name.clone())
            .collect();
        if actual != expected {
            return Err(io::Error::from_raw_os_error(libc::EBUSY));
        }
        for (name, child) in &snapshot.children {
            verify_snapshot(&path.join(name), child)?;
        }
    }
    Ok(())
}

fn remove_snapshot_at(parent: &File, name: &OsStr, snapshot: &Snapshot) -> io::Result<()> {
    let path = PathBuf::from(format!("/proc/self/fd/{}", parent.as_raw_fd())).join(name);
    let current = fs::symlink_metadata(&path)?;
    // Removing an earlier hardlink changes ctime on its remaining names. Content
    // identity/size/mtime still have to match the completely verified snapshot.
    if !same_identity(&snapshot.metadata, &current)
        || snapshot.metadata.len() != current.len()
        || snapshot.metadata.mtime() != current.mtime()
        || snapshot.metadata.mtime_nsec() != current.mtime_nsec()
    {
        return Err(io::Error::from_raw_os_error(libc::EBUSY));
    }
    if snapshot.metadata.is_dir() {
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)?;
        if !same_identity(&snapshot.metadata, &directory.metadata()?) {
            return Err(io::Error::from_raw_os_error(libc::EBUSY));
        }
        if current.uid() == unsafe { libc::geteuid() } && current.mode() & 0o700 != 0o700 {
            // Match the reference cross-filesystem move for the user's own read-only
            // directories. The open descriptor prevents following a swapped symlink.
            unsafe {
                libc::fchmod(directory.as_raw_fd(), (current.mode() | 0o700) & 0o7777);
            }
        }
        for (child_name, child) in &snapshot.children {
            remove_snapshot_at(&directory, child_name, child)?;
        }
    }
    let name =
        CString::new(name.as_bytes()).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?;
    let result = unsafe {
        libc::unlinkat(
            parent.as_raw_fd(),
            name.as_ptr(),
            if snapshot.metadata.is_dir() {
                libc::AT_REMOVEDIR
            } else {
                0
            },
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("win11-native-mutations-{}", nonce()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn progress() -> Progress<'static> {
        Progress {
            ctx: None,
            job: "test".into(),
            moving: false,
            total: None,
            done: 0,
            items: 0,
            count: 1,
            current: String::new(),
            last: None,
        }
    }

    #[test]
    fn conflict_names_preserve_compound_extensions_and_broken_links() {
        let dir = Fixture::new();
        fs::write(dir.0.join("a.tar.gz"), "original").unwrap();
        symlink("missing", dir.0.join("a (2).tar.gz")).unwrap();
        assert_eq!(
            unique_path(&dir.0, "a.tar.gz", false).unwrap(),
            dir.0.join("a (3).tar.gz")
        );
        assert_eq!(
            copy_path(&dir.0, "a.tar.gz", false, "Copy").unwrap(),
            dir.0.join("a - Copy.tar.gz")
        );
        assert_eq!(split_name(".bashrc"), (".bashrc", ""));
        assert_eq!(split_name("...hidden"), ("...hidden", ""));
    }
    #[test]
    fn atomic_rename_never_replaces_files_directories_or_dangling_links() {
        let dir = Fixture::new();
        let source = dir.0.join("source");
        fs::write(&source, "valuable").unwrap();
        for name in ["file", "folder", "link"] {
            let target = dir.0.join(name);
            match name {
                "file" => fs::write(&target, "keep").unwrap(),
                "folder" => fs::create_dir(&target).unwrap(),
                _ => symlink("missing", &target).unwrap(),
            }
            assert_eq!(
                rename_noreplace(&source, &target).unwrap_err().kind(),
                io::ErrorKind::AlreadyExists
            );
            assert_eq!(fs::read_to_string(&source).unwrap(), "valuable");
        }
        assert_eq!(fs::read_to_string(dir.0.join("file")).unwrap(), "keep");
    }
    #[test]
    fn recursive_copy_preserves_links_without_entering_link_cycles_or_external_trees() {
        let dir = Fixture::new();
        let source = dir.0.join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("content"), b"original").unwrap();
        symlink(".", source.join("cycle")).unwrap();
        symlink("../elsewhere", source.join("outside")).unwrap();
        let target = dir.0.join("copied");
        let snapshot = copy_any(&source, &target, &mut progress(), 0).unwrap();
        assert_eq!(fs::read(target.join("content")).unwrap(), b"original");
        assert_eq!(
            fs::read_link(target.join("cycle")).unwrap(),
            PathBuf::from(".")
        );
        assert_eq!(
            fs::read_link(target.join("outside")).unwrap(),
            PathBuf::from("../elsewhere")
        );
        assert_eq!(snapshot.children.len(), 3);
    }
    #[test]
    fn failed_copy_keeps_existing_destination_and_source() {
        let dir = Fixture::new();
        let source = dir.0.join("source");
        let target = dir.0.join("target");
        fs::write(&source, b"source").unwrap();
        fs::write(&target, b"keep").unwrap();
        assert_eq!(
            copy_any(&source, &target, &mut progress(), 0)
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(target).unwrap(), b"keep");
        assert_eq!(fs::read(source).unwrap(), b"source");
    }
    #[test]
    fn changed_cross_filesystem_source_is_not_deleted_after_copy() {
        let dir = Fixture::new();
        let source = dir.0.join("source");
        let target = dir.0.join("target");
        fs::write(&source, b"before").unwrap();
        let snapshot = copy_any(&source, &target, &mut progress(), 0).unwrap();
        fs::write(&source, b"a changed source").unwrap();
        assert!(verify_snapshot(&source, &snapshot).is_err());
        assert_eq!(fs::read(&source).unwrap(), b"a changed source");
        assert_eq!(fs::read(target).unwrap(), b"before");
    }
    #[test]
    fn removal_does_not_follow_replaced_child_links() {
        let dir = Fixture::new();
        let source = dir.0.join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("child"), b"copied").unwrap();
        let target = dir.0.join("target");
        let snapshot = copy_any(&source, &target, &mut progress(), 0).unwrap();
        let outside = dir.0.join("outside");
        fs::write(&outside, b"keep").unwrap();
        fs::remove_file(source.join("child")).unwrap();
        symlink(&outside, source.join("child")).unwrap();
        let parent = File::open(&dir.0).unwrap();
        assert!(remove_snapshot_at(&parent, OsStr::new("source"), &snapshot).is_err());
        assert_eq!(fs::read(outside).unwrap(), b"keep");
    }
    #[test]
    fn transfer_reports_partial_success_without_losing_either_source() {
        let dir = Fixture::new();
        let source = dir.0.join("good");
        fs::write(&source, b"good").unwrap();
        let missing = dir.0.join("missing");
        let dest = dir.0.join("dest");
        fs::create_dir(&dest).unwrap();
        let ctx = Context::new(dir.0.clone(), dir.0.join("Desktop"), "en".into(), false);
        let result = transfer(
            vec![source.clone(), missing],
            dest.clone(),
            false,
            Some("xp-original-job"),
            &ctx,
        )
        .unwrap();
        assert_eq!(result["ok"], false);
        assert_eq!(result["jobId"], "xp-original-job");
        assert_eq!(result["paths"].as_array().unwrap().len(), 1);
        assert_eq!(result["failed"].as_array().unwrap().len(), 1);
        // failed[].error: io_error's sentence with the quoted name, in the context's language (en)
        assert_eq!(result["failed"][0]["error"], "“missing” no longer exists.");
        assert_eq!(fs::read(source).unwrap(), b"good");
        assert_eq!(fs::read(dest.join("good")).unwrap(), b"good");
    }
    #[test]
    fn descendant_destination_through_a_link_is_rejected() {
        let dir = Fixture::new();
        let source = dir.0.join("source");
        fs::create_dir(&source).unwrap();
        let inside = source.join("inside");
        fs::create_dir(&inside).unwrap();
        let alias = dir.0.join("alias");
        symlink(&inside, &alias).unwrap();
        let ctx = Context::new(dir.0.clone(), dir.0.join("Desktop"), "en".into(), false);
        let result = transfer(vec![source.clone()], alias, false, None, &ctx).unwrap();
        assert_eq!(result["ok"], false);
        assert!(result["paths"].as_array().unwrap().is_empty());
        // the into-itself marker's text (files.intoItself) is the failed[] text, in en
        assert_eq!(
            result["failed"][0]["error"],
            "You can’t put a folder inside itself."
        );
        assert!(fs::read_dir(inside).unwrap().next().is_none());
        assert!(source.is_dir());
    }
    #[test]
    fn same_folder_copy_localizes_name_and_move_is_a_noop() {
        let dir = Fixture::new();
        let source = dir.0.join("archive.tar.gz");
        fs::write(&source, b"archive").unwrap();
        let ctx = Context::new(dir.0.clone(), dir.0.join("Desktop"), "nl".into(), false);
        let first = transfer(vec![source.clone()], dir.0.clone(), false, None, &ctx).unwrap();
        assert_eq!(
            first["paths"][0],
            json!(dir.0.join("archive - kopie.tar.gz"))
        );
        let second = transfer(vec![source.clone()], dir.0.clone(), false, None, &ctx).unwrap();
        assert_eq!(
            second["paths"][0],
            json!(dir.0.join("archive - kopie (2).tar.gz"))
        );
        let moved = transfer(vec![source.clone()], dir.0.clone(), true, None, &ctx).unwrap();
        assert_eq!(moved["paths"][0], json!(source));
        assert_eq!(fs::read(source).unwrap(), b"archive");
    }
    #[test]
    fn cross_device_copy_remove_helper_completes_tree_before_removing_source() {
        let dir = Fixture::new();
        let source = dir.0.join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("first"), b"first").unwrap();
        fs::hard_link(source.join("first"), source.join("hardlink")).unwrap();
        symlink("first", source.join("link")).unwrap();
        let target = dir.0.join("target");
        copy_then_remove(&source, &target, &mut progress()).unwrap();
        assert!(!lexists(&source));
        assert_eq!(fs::read(target.join("first")).unwrap(), b"first");
        assert_eq!(fs::read(target.join("hardlink")).unwrap(), b"first");
        assert_eq!(
            fs::read_link(target.join("link")).unwrap(),
            PathBuf::from("first")
        );
    }
    #[test]
    fn special_files_are_rejected_without_leaving_a_partial_tree() {
        let dir = Fixture::new();
        let source = dir.0.join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("normal"), b"normal").unwrap();
        let fifo = source.join("pipe");
        let name = c_path(&fifo).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let target = dir.0.join("target");
        assert!(copy_any(&source, &target, &mut progress(), 0).is_err());
        assert!(!lexists(&target));
        assert!(lexists(&fifo));
        assert_eq!(fs::read(source.join("normal")).unwrap(), b"normal");
    }
    #[test]
    fn simultaneous_rename_has_exactly_one_winner_and_no_replacement() {
        let dir = Fixture::new();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let target = dir.0.join("shared");
        let mut workers = Vec::new();
        for number in 0..8 {
            let source = dir.0.join(format!("source-{number}"));
            fs::write(&source, number.to_string()).unwrap();
            let target = target.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                (number, source.clone(), rename_noreplace(&source, &target))
            }));
        }
        let results: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(
            results
                .iter()
                .filter(|(_, _, result)| result.is_ok())
                .count(),
            1
        );
        for (number, source, result) in results {
            if result.is_ok() {
                assert!(!source.exists());
                assert_eq!(fs::read_to_string(&target).unwrap(), number.to_string());
            } else {
                assert_eq!(result.unwrap_err().kind(), io::ErrorKind::AlreadyExists);
                assert_eq!(fs::read_to_string(source).unwrap(), number.to_string());
            }
        }
    }
    #[test]
    fn copied_regular_file_retains_mode_modified_time_and_supported_xattrs() {
        use std::os::unix::fs::PermissionsExt;
        let dir = Fixture::new();
        let source = dir.0.join("source");
        fs::write(&source, b"metadata").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o640)).unwrap();
        let input = File::open(&source).unwrap();
        input
            .set_times(fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(12345)))
            .unwrap();
        let attribute = CString::new("user.win11-native-test").unwrap();
        let value = b"retain this";
        let supports_xattr = unsafe {
            libc::fsetxattr(
                input.as_raw_fd(),
                attribute.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
            )
        } == 0;
        let target = dir.0.join("target");
        copy_any(&source, &target, &mut progress(), 0).unwrap();
        let metadata = fs::metadata(&target).unwrap();
        assert_eq!(metadata.mode() & 0o777, 0o640);
        assert_eq!(metadata.mtime(), 12345);
        if supports_xattr {
            let output = File::open(&target).unwrap();
            let mut found = [0_u8; 64];
            let length = unsafe {
                libc::fgetxattr(
                    output.as_raw_fd(),
                    attribute.as_ptr(),
                    found.as_mut_ptr().cast(),
                    found.len(),
                )
            };
            assert_eq!(length, value.len() as isize);
            assert_eq!(&found[..length as usize], value);
        }
    }
    #[test]
    fn regular_file_replaced_with_fifo_is_rejected_before_a_blocking_read() {
        let dir = Fixture::new();
        let source = dir.0.join("source");
        fs::write(&source, b"original").unwrap();
        let expected = fs::symlink_metadata(&source).unwrap();
        fs::remove_file(&source).unwrap();
        let name = c_path(&source).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert_eq!(
            open_regular_source(&source, &expected)
                .unwrap_err()
                .raw_os_error(),
            Some(libc::EBUSY)
        );
    }
}
