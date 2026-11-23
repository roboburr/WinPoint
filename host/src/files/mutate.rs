//! Conflict-safe Explorer writes. Existing names, including dangling links, are never replaced.
use super::{
    ApiError, ApiResult, Context, abs_in, control, directory, io_error, paths, validate_name,
};
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
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
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

#[path = "transfer_interactive.rs"]
mod interactive;
#[path = "transfer_bytes.rs"]
mod transfer_bytes;

pub(super) fn call(method: &str, args: &Value, ctx: &Context) -> Option<ApiResult> {
    match method {
        "files.createFolder" => Some(create(args, ctx, true)),
        "files.createFile" => Some(create(args, ctx, false)),
        "files.rename" => Some(rename(args, ctx)),
        "files.copy" | "files.move" => Some((|| {
            let sources = paths(&args["paths"], ctx.lang())?;
            let dest = directory(&args["dest"], ctx.lang())?;
            let job = args.get("jobId").and_then(Value::as_str);
            transfer_with_options(
                sources,
                dest,
                method == "files.move",
                job,
                args["interactive"] == true,
                ctx,
            )
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

#[derive(Clone, Copy)]
struct MetricMark {
    done: u64,
    entries_done: Option<u64>,
}

struct Progress<'a> {
    ctx: Option<&'a Context>,
    job: String,
    moving: bool,
    total: Option<u64>,
    done: u64,
    items: usize,
    count: usize,
    entry_count: Option<u64>,
    entries_done: Option<u64>,
    current: String,
    last: Option<Instant>,
    control: Option<Arc<control::Job>>,
    cancelled: bool,
    partial_paths: Vec<PathBuf>,
    overwritten_paths: Vec<PathBuf>,
    merged_paths: Vec<PathBuf>,
    skipped_paths: Vec<PathBuf>,
    recovery_paths: Vec<PathBuf>,
    copy_buffer: Vec<u8>,
    #[cfg(test)]
    after_write: Option<Box<dyn FnMut(u64) + Send>>,
    #[cfg(test)]
    before_commit: Option<Box<dyn FnMut() + Send>>,
    #[cfg(test)]
    before_stage: Option<Box<dyn FnMut() + Send>>,
    #[cfg(test)]
    fail_after_write: Option<u64>,
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
            let mut event = json!({"jobId":self.job,"op":if self.moving {"move"} else {"copy"},"done":self.done,"total":self.total,"items":self.items,"count":self.count,"entryCount":self.entry_count,"entriesDone":self.entries_done,"current":self.current,"finished":finished});
            if let Some(control) = &self.control {
                for (key, value) in control.json().as_object().unwrap() {
                    event[key] = value.clone();
                }
            }
            ctx.emit("files.progress", event);
        }
    }
    fn add(&mut self, bytes: u64) {
        self.done = self.done.saturating_add(bytes);
        #[cfg(test)]
        if let Some(hook) = &mut self.after_write {
            hook(self.done);
        }
        self.emit(false, false);
    }
    fn metric_mark(&self) -> MetricMark {
        MetricMark {
            done: self.done,
            entries_done: self.entries_done,
        }
    }
    fn uncertain_metrics(&mut self) {
        self.total = None;
        self.entry_count = None;
        self.entries_done = None;
    }
    fn discard_copy(
        &mut self,
        mark: MetricMark,
        path: &Path,
        cleanup: io::Result<()>,
        source_changed: bool,
    ) {
        let discarded = cleanup.is_ok() && !lexists(path);
        self.note_partial(path, cleanup);
        if discarded {
            self.done = mark.done;
            self.entries_done = mark.entries_done;
        } else {
            self.uncertain_metrics();
        }
        if source_changed {
            self.uncertain_metrics();
        }
        // Native speed samples are reset when done moves backwards; publish this before another attempt.
        self.emit(false, true);
    }
    fn checkpoint(&mut self) -> io::Result<()> {
        if let Some(control) = self.control.clone() {
            control.checkpoint(|| self.emit(false, true))?;
        }
        Ok(())
    }
    fn complete_entries(&mut self, count: Option<u64>) {
        self.entries_done = self
            .entries_done
            .zip(count)
            .map(|(done, count)| done.saturating_add(count));
    }
    fn begin_commit(&mut self) -> io::Result<Option<control::Commit>> {
        #[cfg(test)]
        if let Some(hook) = &mut self.before_commit {
            hook();
        }
        self.control
            .clone()
            .map(|control| control.begin_commit(|| self.emit(false, true)))
            .transpose()
    }
    fn finish(&mut self, phase: &'static str) {
        if let Some(control) = &self.control {
            control.finish(phase);
        }
        self.emit(true, true);
    }
    fn note_partial(&mut self, path: &Path, cleanup: io::Result<()>) {
        if cleanup.is_err() && lexists(path) {
            // Recursive targets use descriptor-relative paths; report the real pathname while the fd lives.
            let reported = path
                .strip_prefix("/proc/self/fd")
                .ok()
                .and_then(|relative| {
                    let mut components = relative.components();
                    let descriptor = components.next()?.as_os_str();
                    let directory =
                        fs::read_link(Path::new("/proc/self/fd").join(descriptor)).ok()?;
                    Some(directory.join(components.as_path()))
                })
                .unwrap_or_else(|| path.into());
            if !self.partial_paths.contains(&reported) {
                self.partial_paths.push(reported);
            }
        }
    }
}

pub(super) fn transfer(
    sources: Vec<PathBuf>,
    dest: PathBuf,
    moving: bool,
    job: Option<&str>,
    ctx: &Context,
) -> ApiResult {
    transfer_with_options(sources, dest, moving, job, false, ctx)
}

pub(super) fn transfer_with_options(
    sources: Vec<PathBuf>,
    dest: PathBuf,
    moving: bool,
    job: Option<&str>,
    interactive: bool,
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
    let job = job
        .filter(|job| !job.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(nonce);
    let control = ctx.transfers.register(&job, ctx)?;
    let mut progress = Progress {
        ctx: Some(ctx),
        job,
        moving,
        total: None,
        done: 0,
        items: 0,
        count: sources.len(),
        entry_count: None,
        entries_done: Some(0),
        current: String::new(),
        last: None,
        control: Some(control),
        cancelled: false,
        partial_paths: Vec::new(),
        overwritten_paths: Vec::new(),
        merged_paths: Vec::new(),
        skipped_paths: Vec::new(),
        recovery_paths: Vec::new(),
        copy_buffer: Vec::new(),
        #[cfg(test)]
        after_write: None,
        #[cfg(test)]
        before_commit: None,
        #[cfg(test)]
        before_stage: None,
        #[cfg(test)]
        fail_after_write: None,
    };
    progress.emit(false, true);
    // Register and emit first: a long directory pre-scan is itself pausable/cancellable.
    let mut source_entries = Vec::new();
    match transfer_size(&sources, &mut progress) {
        Ok(Some(plan)) => {
            progress.total = Some(plan.bytes);
            progress.entry_count = Some(plan.entries);
            source_entries = plan.by_source;
        }
        Ok(None) => {}
        Err(error) if control::is_cancelled(&error) => progress.cancelled = true,
        Err(_) => {}
    }
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
            progress.finish("failed");
            return Err(io_error(error, &dest, lang));
        }
    };
    for (source_index, source) in sources.iter().enumerate() {
        if progress.cancelled {
            break;
        }
        let name = source.file_name().and_then(OsStr::to_str).unwrap_or("");
        progress.current = name.to_owned();
        let result = (|| {
            progress.checkpoint()?;
            if name.is_empty() {
                return Err(io::Error::from_raw_os_error(libc::EINVAL));
            }
            let metadata = fs::symlink_metadata(source)?;
            let same_folder = if interactive {
                source
                    .parent()
                    .and_then(|p| fs::canonicalize(p).ok())
                    .as_ref()
                    == Some(&dest_real)
            } else {
                source.parent() == Some(dest.as_path())
            };
            if metadata.is_dir() && dest_real.starts_with(fs::canonicalize(source)?) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    into_itself.as_str(),
                ));
            }
            if moving && same_folder {
                progress.complete_entries(
                    source_entries
                        .get(source_index)
                        .copied()
                        .or_else(|| (!metadata.is_dir()).then_some(1)),
                );
                return Ok(Some(source.clone()));
            }
            if interactive && !(!moving && same_folder) {
                return interactive::entry(source, &dest_real.join(name), moving, &mut progress, 0);
            }
            for attempt in 0..20 {
                let target = if !moving && same_folder {
                    copy_path(&dest, name, metadata.is_dir(), &copy_word)?
                } else {
                    unique_path(&dest, name, metadata.is_dir())?
                };
                let write = if moving {
                    let commit = progress.begin_commit()?;
                    let renamed = rename_noreplace(source, &target);
                    drop(commit);
                    match renamed {
                        Ok(()) => {
                            progress.add(if metadata.is_dir() { 0 } else { metadata.len() });
                            progress.complete_entries(
                                source_entries
                                    .get(source_index)
                                    .copied()
                                    .or_else(|| (!metadata.is_dir()).then_some(1)),
                            );
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
                    Ok(()) => return Ok(Some(target)),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists && attempt < 19 => {
                        continue;
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(io::Error::from_raw_os_error(libc::EEXIST))
        })();
        match result {
            Ok(Some(path)) => {
                completed.push(path);
                if !(moving && source.parent() == Some(dest.as_path())) {
                    progress.items += 1;
                }
            }
            Ok(None) => {}
            Err(error) => {
                if control::is_cancelled(&error) {
                    progress.cancelled = true;
                    break;
                }
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
    for path in &progress.partial_paths {
        failed.push(json!({"path":path,"error":crate::i18n::tr(lang,"files.transferPartial"),"code":"partial_cleanup"}));
    }
    for path in &progress.recovery_paths {
        failed.push(json!({"path":path,"error":crate::i18n::tr(lang,"files.transferRecovery"),"code":"recovery_kept"}));
    }
    progress.finish(if progress.cancelled {
        "cancelled"
    } else if failed.is_empty() {
        "finished"
    } else {
        "failed"
    });
    ctx.changed(&dest);
    if moving {
        for source in &sources {
            if let Some(parent) = source.parent() {
                ctx.changed(parent);
            }
        }
    }
    Ok(
        json!({"ok":failed.is_empty() && !progress.cancelled,"cancelled":progress.cancelled,"paths":completed,"failed":failed,"partialPaths":progress.partial_paths,"recoveryPaths":progress.recovery_paths,"overwrittenPaths":progress.overwritten_paths,"mergedPaths":progress.merged_paths,"skipped":progress.skipped_paths,"undoable":progress.overwritten_paths.is_empty() && progress.merged_paths.is_empty() && progress.recovery_paths.is_empty(),"jobId":progress.job,"entryCount":progress.entry_count,"entriesDone":progress.entries_done}),
    )
}

struct TransferPlan {
    bytes: u64,
    entries: u64,
    by_source: Vec<u64>,
}

fn transfer_size(
    paths: &[PathBuf],
    progress: &mut Progress<'_>,
) -> io::Result<Option<TransferPlan>> {
    let deadline = Instant::now() + Duration::from_millis(1500);
    let mut plan = TransferPlan {
        bytes: 0,
        entries: 0,
        by_source: vec![0; paths.len()],
    };
    let mut stack = paths
        .iter()
        .cloned()
        .enumerate()
        .map(|(i, p)| (p, i))
        .collect::<Vec<_>>();
    while let Some((path, index)) = stack.pop() {
        progress.checkpoint()?;
        if Instant::now() > deadline {
            return Ok(None);
        }
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        plan.entries = plan.entries.saturating_add(1);
        plan.by_source[index] = plan.by_source[index].saturating_add(1);
        if metadata.is_dir() {
            if let Ok(entries) = fs::read_dir(path) {
                for entry in entries.flatten() {
                    progress.checkpoint()?;
                    if Instant::now() > deadline {
                        return Ok(None);
                    }
                    stack.push((entry.path(), index));
                }
            }
        } else {
            plan.bytes = plan.bytes.saturating_add(metadata.len());
        }
    }
    Ok(Some(plan))
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
    copied: Metadata,
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
    let mark = progress.metric_mark();
    progress.checkpoint()?;
    if depth > 256 {
        return Err(io::Error::from_raw_os_error(libc::ELOOP));
    }
    let metadata = fs::symlink_metadata(source)?;
    let mut snapshot = Snapshot {
        metadata: metadata.clone(),
        copied: metadata.clone(),
        children: Vec::new(),
    };
    if metadata.file_type().is_symlink() {
        let link = fs::read_link(source)?;
        if !unchanged(&metadata, &fs::symlink_metadata(source)?) {
            return Err(io::Error::from_raw_os_error(libc::EBUSY));
        }
        symlink(link, target)?;
        let owned = fs::symlink_metadata(target)?;
        snapshot.copied = owned.clone();
        if let Err(error) = progress.checkpoint() {
            let changed = !fs::symlink_metadata(source).is_ok_and(|now| unchanged(&metadata, &now));
            progress.discard_copy(mark, target, cleanup_owned(target, &owned), changed);
            return Err(error);
        }
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
                progress.checkpoint()?;
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
            progress.checkpoint()?;
            preserve_metadata(&directory, &output_directory, &directory.metadata()?);
            snapshot.copied = output_directory.metadata()?;
            if !same_identity(&owned, &fs::symlink_metadata(target)?) {
                return Err(io::Error::from_raw_os_error(libc::EBUSY));
            }
            Ok(())
        })();
        if let Err(error) = result {
            let changed = !fs::symlink_metadata(source).is_ok_and(|now| unchanged(&metadata, &now));
            progress.discard_copy(
                mark,
                target,
                cleanup_tree(target, &owned, &snapshot.children),
                changed,
            );
            return Err(error);
        }
    } else if metadata.is_file() {
        let input = open_regular_source(source, &metadata)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(target)?;
        let mut owned = output.metadata()?;
        // One lazily allocated fallback buffer belongs to the job, not to every file.
        // Namespace, source-consistency and cleanup checks remain here around the byte engine.
        let mut buffer = std::mem::take(&mut progress.copy_buffer);
        let mut engine = transfer_bytes::CopyEngine::new();
        let result = (|| {
            loop {
                progress.checkpoint()?;
                verify_output(&output, target, &owned)?;
                let chunk = engine.copy_next(&input, &output, &mut buffer, || {
                    progress.checkpoint()?;
                    verify_output(&output, target, &owned)
                });
                let written = match &chunk {
                    Ok(chunk) => chunk.bytes_written,
                    Err(error) => error.bytes_written,
                };
                if written != 0 {
                    owned = output.metadata()?;
                    progress.add(written as u64);
                }
                let chunk = chunk.map_err(|error| error.error)?;
                if chunk.eof {
                    break;
                }
                #[cfg(test)]
                if progress
                    .fail_after_write
                    .is_some_and(|limit| progress.done.saturating_sub(mark.done) >= limit)
                {
                    progress.fail_after_write = None;
                    return Err(io::Error::from_raw_os_error(libc::ENOSPC));
                }
            }
            progress.checkpoint()?;
            verify_output(&output, target, &owned)?;
            output.flush()?;
            if !unchanged(&metadata, &input.metadata()?) {
                return Err(io::Error::from_raw_os_error(libc::EBUSY));
            }
            preserve_metadata(&input, &output, &input.metadata()?);
            owned = output.metadata()?;
            snapshot.copied = owned.clone();
            if !same_identity(&owned, &fs::symlink_metadata(target)?) {
                return Err(io::Error::from_raw_os_error(libc::EBUSY));
            }
            Ok(())
        })();
        progress.copy_buffer = buffer;
        if let Err(error) = result {
            drop(output);
            let changed = !fs::symlink_metadata(source).is_ok_and(|now| unchanged(&metadata, &now));
            progress.discard_copy(mark, target, cleanup_owned(target, &owned), changed);
            return Err(error);
        }
    } else {
        return Err(io::Error::from_raw_os_error(libc::EINVAL));
    }
    progress.complete_entries(Some(1));
    Ok(snapshot)
}

fn verify_output(output: &File, path: &Path, expected: &Metadata) -> io::Result<()> {
    if !unchanged(expected, &output.metadata()?)
        || !same_identity(expected, &fs::symlink_metadata(path)?)
    {
        return Err(io::Error::from_raw_os_error(libc::EBUSY));
    }
    Ok(())
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

fn cleanup_owned(path: &Path, owned: &Metadata) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
        Ok(current) if !unchanged(owned, &current) => {
            Err(io::Error::from_raw_os_error(libc::EBUSY))
        }
        Ok(_) => fs::remove_file(path),
    }
}

fn cleanup_tree(
    path: &Path,
    owned: &Metadata,
    children: &[(OsString, Snapshot)],
) -> io::Result<()> {
    let current = match fs::symlink_metadata(path) {
        Ok(current) => current,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !same_identity(owned, &current) {
        return Err(io::Error::from_raw_os_error(libc::EBUSY));
    }
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    if !same_identity(owned, &directory.metadata()?) {
        return Err(io::Error::from_raw_os_error(libc::EBUSY));
    }
    let opened = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
    let mut incomplete = false;
    for (name, snapshot) in children {
        let child = opened.join(name);
        let result = if snapshot.copied.is_dir() {
            cleanup_tree(&child, &snapshot.copied, &snapshot.children)
        } else {
            cleanup_owned(&child, &snapshot.copied)
        };
        incomplete |= result.is_err();
    }
    // rmdir is deliberately nonrecursive: unknown or externally changed entries survive.
    if incomplete || !same_identity(owned, &fs::symlink_metadata(path)?) {
        return Err(io::Error::from_raw_os_error(libc::EBUSY));
    }
    fs::remove_dir(path)
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
    let mark = progress.metric_mark();
    let snapshot = copy_any(source, target, progress, 0)?;
    let prepared = (|| {
        loop {
            progress.checkpoint()?;
            verify_snapshot_checked(source, &snapshot, &mut || progress.checkpoint())?;
            verify_copied_checked(target, &snapshot, &mut || progress.checkpoint())?;
            let commit = progress.begin_commit()?;
            // Admission may have paused. Recheck both exact trees AFTER resume, before deleting anything.
            verify_snapshot_checked(source, &snapshot, &mut || Ok(()))?;
            verify_copied_checked(target, &snapshot, &mut || Ok(()))?;
            if progress.control.as_ref().is_some_and(|job| job.pending()) {
                drop(commit);
                continue;
            }
            break Ok(commit);
        }
    })();
    let commit = match prepared {
        Ok(commit) => commit,
        Err(error) => {
            if control::is_cancelled(&error) {
                let cleanup = if snapshot.copied.is_dir() {
                    cleanup_tree(target, &snapshot.copied, &snapshot.children)
                } else {
                    cleanup_owned(target, &snapshot.copied)
                };
                let changed = !fs::symlink_metadata(source)
                    .is_ok_and(|now| unchanged(&snapshot.metadata, &now));
                progress.discard_copy(mark, target, cleanup, changed);
            }
            return Err(error);
        }
    };
    // Keep both copies rather than remove a source that changed while being copied.
    // From here to the end of removal, finish this one committed move even if cancellation arrives.
    let parent = source
        .parent()
        .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?;
    let name = source
        .file_name()
        .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?;
    let parent = File::open(parent)?;
    let result = remove_snapshot_at(&parent, name, &snapshot);
    drop(commit);
    result
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
                entry_count: None,
                entries_done: Some(0),
                current: String::new(),
                last: None,
                control: None,
                cancelled: false,
                partial_paths: Vec::new(),
                overwritten_paths: Vec::new(),
                merged_paths: Vec::new(),
                skipped_paths: Vec::new(),
                recovery_paths: Vec::new(),
                copy_buffer: Vec::new(),
                #[cfg(test)]
                after_write: None,
                #[cfg(test)]
                before_commit: None,
                #[cfg(test)]
                before_stage: None,
                #[cfg(test)]
                fail_after_write: None,
            };
            copy_then_remove(source, target, &mut progress)
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
fn verify_snapshot(path: &Path, snapshot: &Snapshot) -> io::Result<()> {
    verify_snapshot_checked(path, snapshot, &mut || Ok(()))
}

fn verify_snapshot_checked(
    path: &Path,
    snapshot: &Snapshot,
    checkpoint: &mut impl FnMut() -> io::Result<()>,
) -> io::Result<()> {
    verify_tree_checked(path, snapshot, false, checkpoint)
}

fn verify_copied_checked(
    path: &Path,
    snapshot: &Snapshot,
    checkpoint: &mut impl FnMut() -> io::Result<()>,
) -> io::Result<()> {
    verify_tree_checked(path, snapshot, true, checkpoint)
}

fn verify_tree_checked(
    path: &Path,
    snapshot: &Snapshot,
    copied: bool,
    checkpoint: &mut impl FnMut() -> io::Result<()>,
) -> io::Result<()> {
    checkpoint()?;
    let metadata = if copied {
        &snapshot.copied
    } else {
        &snapshot.metadata
    };
    if !unchanged(metadata, &fs::symlink_metadata(path)?) {
        return Err(io::Error::from_raw_os_error(libc::EBUSY));
    }
    if metadata.is_dir() {
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
            verify_tree_checked(&path.join(name), child, copied, checkpoint)?;
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
    pub(super) fn progress() -> Progress<'static> {
        Progress {
            ctx: None,
            job: "test".into(),
            moving: false,
            total: None,
            done: 0,
            items: 0,
            count: 1,
            entry_count: None,
            entries_done: Some(0),
            current: String::new(),
            last: None,
            control: None,
            cancelled: false,
            partial_paths: Vec::new(),
            overwritten_paths: Vec::new(),
            merged_paths: Vec::new(),
            skipped_paths: Vec::new(),
            recovery_paths: Vec::new(),
            copy_buffer: Vec::new(),
            after_write: None,
            before_commit: None,
            before_stage: None,
            fail_after_write: None,
        }
    }

    fn wait_state(job: &control::Job, expected: &str) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while job.json()["state"] != expected && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(job.json()["state"], expected);
    }

    #[test]
    fn real_copy_pause_stops_bytes_resume_completes_and_cancel_removes_only_owned_partial() {
        for action in ["resume", "cancel-paused", "cancel-active"] {
            let fixture = Fixture::new();
            let source = fixture.0.join("source");
            let target = fixture.0.join("target");
            let existing = fixture.0.join("existing");
            let contents = vec![0x6d; 12 * 1024 * 1024];
            fs::write(&source, &contents).unwrap();
            fs::write(&existing, b"untouched").unwrap();
            let ctx = Context::new(
                fixture.0.clone(),
                fixture.0.join("Desktop"),
                "en".into(),
                false,
            );
            let job = ctx.transfers.register(action, &ctx).unwrap();
            let worker_job = job.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            let worker_source = source.clone();
            let worker_target = target.clone();
            let worker = std::thread::spawn(move || {
                let mut progress = progress();
                progress.control = Some(worker_job.clone());
                let mut triggered = false;
                progress.after_write = Some(Box::new(move |done| {
                    if done > 0 && !triggered {
                        triggered = true;
                        worker_job.request(if action == "cancel-active" {
                            "cancel"
                        } else {
                            "pause"
                        });
                        tx.send(()).unwrap();
                    }
                }));
                copy_any(&worker_source, &worker_target, &mut progress, 0)
            });
            rx.recv_timeout(Duration::from_secs(3)).unwrap();
            if action != "cancel-active" {
                wait_state(&job, "paused");
                let length = fs::metadata(&target).unwrap().len();
                assert!(length > 0 && length < contents.len() as u64);
                assert!(length <= 8 * 1024 * 1024);
                std::thread::sleep(Duration::from_millis(25));
                assert_eq!(fs::metadata(&target).unwrap().len(), length);
                job.request(if action == "resume" {
                    "resume"
                } else {
                    "cancel"
                });
            }
            let result = worker.join().unwrap();
            if action == "resume" {
                result.unwrap();
                assert_eq!(fs::read(&target).unwrap(), contents);
            } else {
                assert!(control::is_cancelled(&result.unwrap_err()));
                assert!(!lexists(&target));
            }
            assert_eq!(fs::read(&source).unwrap(), contents);
            assert_eq!(fs::read(&existing).unwrap(), b"untouched");
        }
    }

    #[test]
    fn cross_device_cancel_at_delete_boundary_keeps_source_and_cleans_new_copy() {
        // Invoke the exact EXDEV path on private files without requiring a second mounted filesystem.
        let fixture = Fixture::new();
        let source = fixture.0.join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("important"), b"source remains complete").unwrap();
        let target = fixture.0.join("new-copy");
        let existing = fixture.0.join("existing");
        fs::write(&existing, b"existing remains").unwrap();
        let ctx = Context::new(
            fixture.0.clone(),
            fixture.0.join("Desktop"),
            "en".into(),
            false,
        );
        let job = ctx.transfers.register("before-delete", &ctx).unwrap();
        let mut progress = progress();
        progress.control = Some(job.clone());
        progress.before_commit = Some(Box::new(move || job.request("cancel")));
        assert!(control::is_cancelled(
            &copy_then_remove(&source, &target, &mut progress).unwrap_err()
        ));
        assert_eq!(
            fs::read(source.join("important")).unwrap(),
            b"source remains complete"
        );
        assert!(!lexists(&target));
        assert_eq!(fs::read(existing).unwrap(), b"existing remains");
    }

    #[test]
    fn cancel_before_copy_does_not_touch_preexisting_target_or_source() {
        let fixture = Fixture::new();
        let source = fixture.0.join("source");
        let target = fixture.0.join("target");
        fs::write(&source, b"source").unwrap();
        fs::write(&target, b"existing target").unwrap();
        let ctx = Context::new(
            fixture.0.clone(),
            fixture.0.join("Desktop"),
            "en".into(),
            false,
        );
        let job = ctx.transfers.register("cancel-before-copy", &ctx).unwrap();
        job.request("cancel");
        let mut progress = progress();
        progress.control = Some(job);
        assert!(control::is_cancelled(
            &copy_any(&source, &target, &mut progress, 0).unwrap_err()
        ));
        assert_eq!(fs::read(source).unwrap(), b"source");
        assert_eq!(fs::read(target).unwrap(), b"existing target");
    }

    #[test]
    fn cross_device_move_keeps_source_when_completed_target_changed_while_paused() {
        for mutation in ["unlink", "truncate", "replace", "child-replace"] {
            let fixture = Fixture::new();
            let source = fixture.0.join("source");
            let target = fixture.0.join("target");
            if mutation == "child-replace" {
                fs::create_dir(&source).unwrap();
                fs::write(source.join("child"), b"precious source").unwrap();
            } else {
                fs::write(&source, b"precious source").unwrap();
            }
            let ctx = Context::new(
                fixture.0.clone(),
                fixture.0.join("Desktop"),
                "en".into(),
                false,
            );
            let job = ctx.transfers.register(mutation, &ctx).unwrap();
            let gate = job.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            let worker_source = source.clone();
            let worker_target = target.clone();
            let worker = std::thread::spawn(move || {
                let mut progress = progress();
                progress.control = Some(gate.clone());
                progress.before_commit = Some(Box::new(move || {
                    gate.request("pause");
                    tx.send(()).unwrap();
                }));
                copy_then_remove(&worker_source, &worker_target, &mut progress)
            });
            rx.recv_timeout(Duration::from_secs(3)).unwrap();
            wait_state(&job, "paused");
            match mutation {
                "unlink" => fs::remove_file(&target).unwrap(),
                "truncate" => fs::write(&target, b"changed").unwrap(),
                "replace" => {
                    fs::remove_file(&target).unwrap();
                    fs::write(&target, b"foreign replacement").unwrap();
                }
                _ => {
                    fs::remove_file(target.join("child")).unwrap();
                    fs::write(target.join("child"), b"foreign replacement").unwrap();
                }
            }
            job.request("resume");
            assert!(worker.join().unwrap().is_err());
            assert_eq!(
                fs::read(if mutation == "child-replace" {
                    source.join("child")
                } else {
                    source.clone()
                })
                .unwrap(),
                b"precious source"
            );
            if mutation == "replace" {
                assert_eq!(fs::read(target).unwrap(), b"foreign replacement");
            }
        }
    }

    #[test]
    fn cancelling_recursive_copy_preserves_foreign_children_and_changed_partial_files() {
        for mutation in ["unknown-child", "replaced-child", "changed-partial"] {
            let fixture = Fixture::new();
            let source = fixture.0.join("source");
            let target = fixture.0.join("target");
            fs::create_dir(&source).unwrap();
            fs::write(source.join("child"), vec![0x67; 12 * 1024 * 1024]).unwrap();
            let ctx = Context::new(
                fixture.0.clone(),
                fixture.0.join("Desktop"),
                "en".into(),
                false,
            );
            let job = ctx.transfers.register(mutation, &ctx).unwrap();
            let gate = job.clone();
            let worker_source = source.clone();
            let worker_target = target.clone();
            let worker = std::thread::spawn(move || {
                let mut progress = progress();
                progress.control = Some(gate.clone());
                let mut triggered = false;
                progress.after_write = Some(Box::new(move |done| {
                    if done > 0 && !triggered {
                        triggered = true;
                        gate.request("pause");
                    }
                }));
                let result = copy_any(&worker_source, &worker_target, &mut progress, 0);
                (result, progress.partial_paths)
            });
            wait_state(&job, "paused");
            let foreign = if mutation == "unknown-child" {
                target.join("foreign")
            } else {
                target.join("child")
            };
            if mutation == "replaced-child" {
                fs::remove_file(&foreign).unwrap();
            }
            fs::write(&foreign, b"foreign data must survive").unwrap();
            job.request("cancel");
            let (result, partial) = worker.join().unwrap();
            assert!(control::is_cancelled(&result.unwrap_err()));
            assert_eq!(fs::read(&foreign).unwrap(), b"foreign data must survive");
            assert!(target.is_dir());
            assert!(!partial.is_empty());
            assert_eq!(
                fs::metadata(source.join("child")).unwrap().len(),
                12 * 1024 * 1024
            );
        }
    }

    #[test]
    fn resuming_changed_partial_cross_device_copy_never_deletes_its_source() {
        let fixture = Fixture::new();
        let source = fixture.0.join("source");
        let target = fixture.0.join("target");
        let original = vec![0x67; 12 * 1024 * 1024];
        fs::write(&source, &original).unwrap();
        let ctx = Context::new(
            fixture.0.clone(),
            fixture.0.join("Desktop"),
            "en".into(),
            false,
        );
        let job = ctx
            .transfers
            .register("changed-partial-resume", &ctx)
            .unwrap();
        let gate = job.clone();
        let worker_source = source.clone();
        let worker_target = target.clone();
        let worker = std::thread::spawn(move || {
            let mut progress = progress();
            progress.control = Some(gate.clone());
            let mut triggered = false;
            progress.after_write = Some(Box::new(move |done| {
                if done > 0 && !triggered {
                    triggered = true;
                    gate.request("pause");
                }
            }));
            let result = copy_then_remove(&worker_source, &worker_target, &mut progress);
            (result, progress.partial_paths)
        });
        wait_state(&job, "paused");
        fs::write(&target, b"foreign rewrite").unwrap();
        job.request("resume");
        let (result, partial) = worker.join().unwrap();
        assert_eq!(result.unwrap_err().raw_os_error(), Some(libc::EBUSY));
        assert_eq!(fs::read(source).unwrap(), original);
        assert_eq!(fs::read(&target).unwrap(), b"foreign rewrite");
        assert_eq!(partial, vec![target]);
    }

    #[test]
    fn cancelled_transfer_finishes_with_real_state_and_cut_clipboard_is_kept() {
        let fixture = Fixture::new();
        let source = fixture.0.join("source");
        let dest = fixture.0.join("dest");
        fs::write(&source, b"valuable").unwrap();
        fs::create_dir(&dest).unwrap();
        let service = super::super::Files::for_tests(&fixture.0.join("service"));
        let controls = service.transfers.clone();
        let events_seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = events_seen.clone();
        let context = Context::new(
            fixture.0.clone(),
            fixture.0.join("Desktop"),
            "en".into(),
            false,
        );
        service.events.on(
            "files.progress",
            Arc::new(move |event| {
                seen.lock().unwrap().push(event.clone());
                if event["finished"] != true {
                    controls
                        .call(
                            &json!({"jobId":event["jobId"],"action":"cancel"}),
                            &context,
                            false,
                        )
                        .unwrap();
                }
            }),
        );
        let clipboard =
            super::super::clipboard::Clipboard::new(fixture.0.join("clipboard")).unwrap();
        let context = service.context();
        clipboard
            .call(
                "files.clipboardSet",
                &json!({"paths":[source],"op":"cut"}),
                &context,
            )
            .unwrap()
            .unwrap();
        let result = clipboard
            .call(
                "files.paste",
                &json!({"dest":dest,"jobId":"cut-cancel"}),
                &context,
            )
            .unwrap()
            .unwrap();
        assert_eq!(result["cancelled"], true);
        assert_eq!(result["ok"], false);
        assert!(source.exists());
        assert!(fs::read_dir(dest).unwrap().next().is_none());
        assert_eq!(
            clipboard
                .call("files.clipboardGet", &json!({}), &context)
                .unwrap()
                .unwrap()["op"],
            "cut"
        );
        let seen = events_seen.lock().unwrap();
        let terminal = seen.last().unwrap();
        assert_eq!(terminal["state"], "cancelled");
        assert_eq!(terminal["finished"], true);
        assert_eq!(terminal["canCancel"], false);
        service.events.shutdown();
    }

    #[test]
    fn recursive_entry_counts_are_additive_and_include_atomic_directory_moves() {
        for moving in [false, true] {
            let fixture = Fixture::new();
            let source = fixture.0.join("source");
            let dest = fixture.0.join("dest");
            fs::create_dir(&source).unwrap();
            fs::create_dir(source.join("nested")).unwrap();
            fs::create_dir(&dest).unwrap();
            fs::write(source.join("first"), b"123").unwrap();
            fs::write(source.join("second"), b"456").unwrap();
            fs::write(source.join("nested/third"), b"789").unwrap();
            let ctx = Context::new(
                fixture.0.clone(),
                fixture.0.join("Desktop"),
                "en".into(),
                false,
            );
            let result = transfer(
                vec![source.clone()],
                dest.clone(),
                moving,
                Some("recursive-items"),
                &ctx,
            )
            .unwrap();
            assert_eq!(result["ok"], true);
            assert_eq!(result["entryCount"], 5); // selected folder, nested folder and three files
            assert_eq!(result["entriesDone"], 5);
            assert_eq!(fs::read(dest.join("source/nested/third")).unwrap(), b"789");
            assert_eq!(source.exists(), !moving);
        }
    }

    #[test]
    fn completed_tree_cleanup_never_recurses_into_unknown_or_replaced_children() {
        let fixture = Fixture::new();
        let source = fixture.0.join("source");
        let target = fixture.0.join("target");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("owned"), b"owned").unwrap();
        fs::write(source.join("replaced"), b"copied").unwrap();
        let snapshot = copy_any(&source, &target, &mut progress(), 0).unwrap();
        fs::remove_file(target.join("replaced")).unwrap();
        fs::write(target.join("replaced"), b"foreign replacement").unwrap();
        fs::write(target.join("foreign"), b"foreign insertion").unwrap();
        assert!(cleanup_tree(&target, &snapshot.copied, &snapshot.children).is_err());
        assert!(!target.join("owned").exists());
        assert_eq!(
            fs::read(target.join("replaced")).unwrap(),
            b"foreign replacement"
        );
        assert_eq!(
            fs::read(target.join("foreign")).unwrap(),
            b"foreign insertion"
        );
        assert_eq!(fs::read(source.join("replaced")).unwrap(), b"copied");
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
