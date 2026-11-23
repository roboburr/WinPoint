//! Explorer's opt-in collision questions. Legacy callers retain their automatic unique-name contract.
//! Existing directories are merged, never recursively replaced. Every replacement is consent-bound.
use super::*;
use control::Choice;

fn display_path(path: &Path) -> PathBuf {
    path.strip_prefix("/proc/self/fd")
        .ok()
        .and_then(|relative| {
            let mut components = relative.components();
            let descriptor = components.next()?.as_os_str();
            Some(
                fs::read_link(Path::new("/proc/self/fd").join(descriptor))
                    .ok()?
                    .join(components.as_path()),
            )
        })
        .unwrap_or_else(|| path.into())
}

fn kind(metadata: &Metadata) -> &'static str {
    if metadata.file_type().is_symlink() {
        "link"
    } else if metadata.is_dir() {
        "folder"
    } else if metadata.is_file() {
        "file"
    } else {
        "other"
    }
}

fn descriptor(path: &Path, metadata: &Metadata) -> Value {
    let path = display_path(path);
    json!({"path":path,"name":path.file_name().unwrap_or_default().to_string_lossy(),
        "kind":kind(metadata),"size":metadata.len(),
        "mtime":metadata.mtime().saturating_mul(1000).saturating_add(metadata.mtime_nsec()/1_000_000)})
}

fn opened_directory(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}

fn fd_path(file: &File) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()))
}

fn record(paths: &mut Vec<PathBuf>, path: &Path) {
    let path = display_path(path);
    if !paths.contains(&path) {
        paths.push(path);
    }
}

fn ask(
    source: &Path,
    target: &Path,
    source_meta: &Metadata,
    target_meta: &Metadata,
    progress: &mut Progress<'_>,
    force: bool,
) -> io::Result<Choice> {
    progress.checkpoint()?;
    let job = progress
        .control
        .clone()
        .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?;
    let choices: &[&str] = if target_meta.is_dir() {
        &["skip", "keepBoth", "cancel"]
    } else {
        &["replace", "skip", "keepBoth", "cancel"]
    };
    let payload = json!({"kind":"collision","source":descriptor(source,source_meta),"target":descriptor(target,target_meta)});
    let result = job.conflict(payload, choices, force, |payload| {
        progress.emit(false, true);
        if let Some(ctx) = progress.ctx {
            ctx.emit("files.conflict", payload);
        }
    });
    progress.emit(false, true);
    result
}

/// A single logical source; `None` means an explicit Skip. Returned paths are never fd aliases.
pub(super) fn entry(
    source: &Path,
    target: &Path,
    moving: bool,
    progress: &mut Progress<'_>,
    depth: usize,
) -> io::Result<Option<PathBuf>> {
    for _ in 0..64 {
        progress.checkpoint()?;
        let before_source = fs::symlink_metadata(source).ok();
        let before_target = fs::symlink_metadata(target).ok();
        let recoveries = progress.recovery_paths.len();
        match entry_once(source, target, moving, progress, depth) {
            Ok(result) => return Ok(result),
            Err(error) if control::is_cancelled(&error) => return Err(error),
            Err(error) => {
                let recoverable = matches!(
                    error.raw_os_error(),
                    Some(
                        libc::EACCES
                            | libc::EPERM
                            | libc::ENOSPC
                            | libc::EIO
                            | libc::EBUSY
                            | libc::ETXTBSY
                            | libc::EROFS
                    )
                );
                let safe = recoverable
                    && before_source.as_ref().is_some_and(|m| {
                        fs::symlink_metadata(source).is_ok_and(|now| unchanged(m, &now))
                    })
                    && match (&before_target, fs::symlink_metadata(target)) {
                        (Some(m), Ok(now)) => unchanged(m, &now),
                        (None, Err(error)) => error.kind() == io::ErrorKind::NotFound,
                        _ => false,
                    }
                    && progress.recovery_paths.len() == recoveries;
                let choices: &[&str] = if safe {
                    &["retry", "skip", "cancel"]
                } else {
                    &["skip", "cancel"]
                };
                let Some(job) = progress.control.clone() else {
                    return Err(error);
                };
                let copied_error = error
                    .raw_os_error()
                    .map(io::Error::from_raw_os_error)
                    .unwrap_or_else(|| io::Error::new(error.kind(), error.to_string()));
                let lang = progress
                    .ctx
                    .map(|ctx| ctx.lang())
                    .unwrap_or(crate::i18n::Lang::parse("en"));
                let translated = io_error(copied_error, source, lang);
                let info = |path: &Path, metadata: &Option<Metadata>| {
                    metadata.as_ref().map(|m|descriptor(path,m))
                    .unwrap_or_else(||json!({"path":display_path(path),"name":path.file_name().unwrap_or_default().to_string_lossy(),"kind":"unknown","size":null,"mtime":null}))
                };
                let payload = json!({"kind":"error","source":info(source,&before_source),"target":info(target,&before_target),
                    "error":{"code":translated.code,"message":translated.message},"retrySafe":safe});
                let answer = job.conflict(payload, choices, true, |payload| {
                    progress.emit(false, true);
                    if let Some(ctx) = progress.ctx {
                        ctx.emit("files.conflict", payload);
                    }
                })?;
                progress.emit(false, true);
                match answer {
                    Choice::Retry if safe => continue,
                    Choice::Skip => {
                        record(&mut progress.skipped_paths, source);
                        progress.complete_entries(
                            (!before_source.as_ref().is_some_and(|m| m.is_dir())).then_some(1),
                        );
                        return Ok(None);
                    }
                    _ => return Err(control::cancelled()),
                }
            }
        }
    }
    Err(io::Error::from_raw_os_error(libc::EBUSY))
}

fn entry_once(
    source: &Path,
    target: &Path,
    moving: bool,
    progress: &mut Progress<'_>,
    depth: usize,
) -> io::Result<Option<PathBuf>> {
    if depth > 256 {
        return Err(io::Error::from_raw_os_error(libc::ELOOP));
    }
    let parent = target
        .parent()
        .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?;
    let parent = opened_directory(&fs::canonicalize(parent)?)?;
    let target = fd_path(&parent).join(
        target
            .file_name()
            .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?,
    );
    let mut force_question = false;
    for _ in 0..64 {
        progress.checkpoint()?;
        let source_meta = fs::symlink_metadata(source)?;
        progress.current = source
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let target_meta = match fs::symlink_metadata(&target) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match create(source, &target, moving, progress) {
                    Ok(()) => return Ok(Some(display_path(&target))),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        force_question = true;
                        continue;
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(error) => return Err(error),
        };
        if source_meta.is_dir() && target_meta.is_dir() {
            merge(
                source,
                &target,
                &source_meta,
                &target_meta,
                moving,
                progress,
                depth,
            )?;
            return Ok(Some(display_path(&target)));
        }
        let choice = ask(
            source,
            &target,
            &source_meta,
            &target_meta,
            progress,
            force_question,
        )?;
        progress.checkpoint()?;
        match choice {
            Choice::Cancel => return Err(control::cancelled()),
            Choice::Skip => {
                record(&mut progress.skipped_paths, source);
                progress.complete_entries((!source_meta.is_dir()).then_some(1));
                return Ok(None);
            }
            Choice::KeepBoth => {
                for _ in 0..20 {
                    let next = unique_path(
                        &fd_path(&parent),
                        target
                            .file_name()
                            .unwrap()
                            .to_str()
                            .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?,
                        source_meta.is_dir(),
                    )?;
                    match create(source, &next, moving, progress) {
                        Ok(()) => return Ok(Some(display_path(&next))),
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                        Err(error) => return Err(error),
                    }
                }
                return Err(io::Error::from_raw_os_error(libc::EEXIST));
            }
            Choice::Replace => {
                // An answer authorises this observed source/target, not a replacement raced into that pathname.
                if !fs::symlink_metadata(source).is_ok_and(|m| unchanged(&source_meta, &m))
                    || !fs::symlink_metadata(&target).is_ok_and(|m| unchanged(&target_meta, &m))
                {
                    force_question = true;
                    continue;
                }
                match replace(
                    source,
                    &target,
                    &source_meta,
                    &target_meta,
                    moving,
                    progress,
                ) {
                    Ok(true) => return Ok(Some(display_path(&target))),
                    Ok(false) => {
                        force_question = true;
                        continue;
                    }
                    Err(error) => return Err(error),
                }
            }
            Choice::Retry => return Err(io::Error::from_raw_os_error(libc::EINVAL)),
        }
    }
    Err(io::Error::from_raw_os_error(libc::EBUSY))
}

fn create(
    source: &Path,
    target: &Path,
    moving: bool,
    progress: &mut Progress<'_>,
) -> io::Result<()> {
    if !moving {
        return copy_any(source, target, progress, 0).map(|_| ());
    }
    let metadata = fs::symlink_metadata(source)?;
    let commit = progress.begin_commit()?;
    let renamed = rename_noreplace(source, target);
    drop(commit);
    match renamed {
        Ok(()) => {
            progress.add(if metadata.is_dir() { 0 } else { metadata.len() });
            // For a directory no byte traversal took place, so never invent its recursive item count.
            progress.complete_entries((!metadata.is_dir()).then_some(1));
            Ok(())
        }
        Err(error) if error.raw_os_error() == Some(libc::EXDEV) || unsupported_rename(&error) => {
            copy_then_remove(source, target, progress)
        }
        Err(error) => Err(error),
    }
}

fn merge(
    source: &Path,
    target: &Path,
    source_meta: &Metadata,
    target_meta: &Metadata,
    moving: bool,
    progress: &mut Progress<'_>,
    depth: usize,
) -> io::Result<()> {
    let input = opened_directory(source)?;
    let output = opened_directory(target)?;
    if !same_identity(source_meta, &input.metadata()?)
        || !same_identity(target_meta, &output.metadata()?)
    {
        return Err(io::Error::from_raw_os_error(libc::EBUSY));
    }
    record(&mut progress.merged_paths, target); // disables destructive top-level Undo even on a partial result
    for child in fs::read_dir(fd_path(&input))? {
        progress.checkpoint()?;
        if !fs::symlink_metadata(target).is_ok_and(|m| same_identity(target_meta, &m)) {
            return Err(io::Error::from_raw_os_error(libc::EBUSY));
        }
        let child = child?;
        entry(
            &child.path(),
            &fd_path(&output).join(child.file_name()),
            moving,
            progress,
            depth + 1,
        )?;
    }
    progress.checkpoint()?;
    if moving {
        let commit = progress.begin_commit()?;
        if !fs::symlink_metadata(source).is_ok_and(|m| same_identity(source_meta, &m)) {
            return Err(io::Error::from_raw_os_error(libc::EBUSY));
        }
        // Skip or a concurrently added source entry leaves its directory intact; never recursively delete it.
        match fs::remove_dir(source) {
            Ok(()) => {}
            Err(error) if error.raw_os_error() == Some(libc::ENOTEMPTY) => {}
            Err(error) => return Err(error),
        }
        drop(commit);
    }
    progress.complete_entries(Some(1));
    Ok(())
}

/// rename changes ctime itself; validate every other captured identity/content attribute after capture.
fn captured_matches(expected: &Metadata, actual: &Metadata) -> bool {
    same_identity(expected, actual)
        && expected.len() == actual.len()
        && expected.mtime() == actual.mtime()
        && expected.mtime_nsec() == actual.mtime_nsec()
        && expected.mode() == actual.mode()
        && expected.uid() == actual.uid()
        && expected.gid() == actual.gid()
}

fn unsupported_rename(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::EINVAL | libc::ENOSYS | libc::EOPNOTSUPP)
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Publication {
    Renamed,
    Linked,
}

/// Explorer-only namespace operations. The general no-replace helper remains strict.
trait TransferNames {
    fn no_replace(&mut self, source: &Path, target: &Path) -> io::Result<()>;
    fn capture_slot(&mut self, source: &Path, target: &Path) -> io::Result<()> {
        fs::rename(source, target)
    }
    fn link(&mut self, source: &Path, target: &Path) -> io::Result<()> {
        let source = c_path(source)?;
        let target = c_path(target)?;
        // Flags zero links a symlink itself, never its referent, and fails atomically if target exists.
        let result = unsafe {
            libc::linkat(
                libc::AT_FDCWD,
                source.as_ptr(),
                libc::AT_FDCWD,
                target.as_ptr(),
                0,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    fn remove_alias(&mut self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }
    #[cfg(test)]
    fn reserved_slot(&mut self, _slot: &Path) -> io::Result<()> {
        Ok(())
    }
}

impl<F: FnMut(&Path, &Path) -> io::Result<()>> TransferNames for F {
    fn no_replace(&mut self, source: &Path, target: &Path) -> io::Result<()> {
        self(source, target)
    }
}

fn exclusive_stage(directory: &File) -> io::Result<()> {
    let metadata = directory.metadata()?;
    // NTFS/FUSE may expose requested0700 as0755. Confidentiality is not assumed here:
    // this transaction needs ownership and exclusive write access to its private slots.
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o700 != 0o700
        || metadata.mode() & 0o022 != 0
    {
        return Err(io::Error::from_raw_os_error(libc::EPERM));
    }
    Ok(())
}

/// Capture a public entry into an exclusively writable, caller-owned stage. An ordinary
/// rename can replace only our still-open reserve file, never a public destination.
/// No post-capture syscall is hidden here: Ok immediately tells the caller it owns the capture.
fn capture_private(
    source: &Path,
    slot: &Path,
    expected: &Metadata,
    names: &mut impl TransferNames,
) -> io::Result<()> {
    match names.no_replace(source, slot) {
        Ok(()) => return Ok(()),
        Err(error) if unsupported_rename(&error) => {}
        Err(error) => return Err(error),
    }
    if !(expected.is_file() || expected.file_type().is_symlink()) {
        return Err(io::Error::from_raw_os_error(libc::EXDEV));
    }
    // The parent is an internal /proc/self/fd/<stage> alias held open by the caller.
    let directory = File::open(
        slot.parent()
            .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?,
    )?;
    exclusive_stage(&directory)?;
    let reserve = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(slot)?;
    let owned = reserve.metadata()?;
    let result = (|| {
        #[cfg(test)]
        names.reserved_slot(slot)?;
        exclusive_stage(&directory)?;
        if !unchanged(&owned, &reserve.metadata()?)
            || !fs::symlink_metadata(slot).is_ok_and(|now| unchanged(&owned, &now))
            || !fs::symlink_metadata(source).is_ok_and(|now| captured_matches(expected, &now))
        {
            return Err(io::Error::from_raw_os_error(libc::EBUSY));
        }
        names.capture_slot(source, slot)
    })();
    if result.is_err() && fs::symlink_metadata(slot).is_ok_and(|now| unchanged(&owned, &now)) {
        // A replaced foreign slot is never removed, even on a failed capture.
        let _ = names.remove_alias(slot);
    }
    result
}

/// Publishing never unlinks the private alias. The caller records Linked immediately,
/// verifies the public entry, then either removes its private alias or reports recovery.
fn publish_private(
    source: &Path,
    target: &Path,
    expected: &Metadata,
    names: &mut impl TransferNames,
) -> io::Result<Publication> {
    match names.no_replace(source, target) {
        Ok(()) => return Ok(Publication::Renamed),
        Err(error) if unsupported_rename(&error) => {}
        Err(error) => return Err(error),
    }
    if !(expected.is_file() || expected.file_type().is_symlink()) {
        return Err(io::Error::from_raw_os_error(libc::EXDEV));
    }
    if !fs::symlink_metadata(source).is_ok_and(|now| captured_matches(expected, &now)) {
        return Err(io::Error::from_raw_os_error(libc::EBUSY));
    }
    names.link(source, target)?;
    Ok(Publication::Linked)
}

fn cleanup_alias(path: &Path, owned: &Metadata, names: &mut impl TransferNames) -> io::Result<()> {
    if !fs::symlink_metadata(path).is_ok_and(|now| captured_matches(owned, &now)) {
        return Err(io::Error::from_raw_os_error(libc::EBUSY));
    }
    names.remove_alias(path)
}

fn restore_private(
    source: &Path,
    target: &Path,
    owned: &Metadata,
    captured: &mut bool,
    names: &mut impl TransferNames,
) -> bool {
    let publication = match publish_private(source, target, owned, names) {
        Ok(publication) => publication,
        Err(_) => return false,
    };
    *captured = publication == Publication::Linked;
    if !fs::symlink_metadata(target).is_ok_and(|now| captured_matches(owned, &now)) {
        return false;
    }
    if *captured && cleanup_alias(source, owned, names).is_ok() {
        *captured = false;
    }
    !*captured
}

fn cleanup_snapshot(path: &Path, snapshot: &Snapshot) -> io::Result<()> {
    if snapshot.copied.is_dir() {
        cleanup_tree(path, &snapshot.copied, &snapshot.children)
    } else {
        cleanup_owned(path, &snapshot.copied)
    }
}

fn remove_source(source: &Path, target: &Path, snapshot: &Snapshot) -> io::Result<()> {
    verify_snapshot_checked(source, snapshot, &mut || Ok(()))?;
    verify_copied_checked(target, snapshot, &mut || Ok(()))?;
    let parent = File::open(
        source
            .parent()
            .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?,
    )?;
    remove_snapshot_at(
        &parent,
        source
            .file_name()
            .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?,
        snapshot,
    )
}

fn replace(
    source: &Path,
    target: &Path,
    consent_source: &Metadata,
    expected: &Metadata,
    moving: bool,
    progress: &mut Progress<'_>,
) -> io::Result<bool> {
    // Existing directories never arrive here; a link is moved as a link, not followed.
    if expected.is_dir() {
        return Err(io::Error::from_raw_os_error(libc::EISDIR));
    }
    if moving
        && let Some(replaced) = replace_move(source, target, consent_source, expected, progress)?
    {
        return Ok(replaced);
    }
    replace_copy(source, target, consent_source, expected, moving, progress)
}

fn replace_move(
    source: &Path,
    target: &Path,
    consent_source: &Metadata,
    expected: &Metadata,
    progress: &mut Progress<'_>,
) -> io::Result<Option<bool>> {
    replace_move_with(
        source,
        target,
        consent_source,
        expected,
        progress,
        rename_noreplace,
    )
}

/// `None` leaves both public entries untouched and selects the existing copy/verify/delete path.
/// The injected rename is the same no-replace syscall in production; tests exercise its race/error boundaries.
fn replace_move_with(
    source: &Path,
    target: &Path,
    consent_source: &Metadata,
    expected: &Metadata,
    progress: &mut Progress<'_>,
    mut rename: impl FnMut(&Path, &Path) -> io::Result<()>,
) -> io::Result<Option<bool>> {
    replace_move_using(
        source,
        target,
        consent_source,
        expected,
        progress,
        &mut rename,
    )
}

fn replace_move_using(
    source: &Path,
    target: &Path,
    consent_source: &Metadata,
    expected: &Metadata,
    progress: &mut Progress<'_>,
    names: &mut impl TransferNames,
) -> io::Result<Option<bool>> {
    if !(consent_source.is_file() || consent_source.file_type().is_symlink()) || expected.is_dir() {
        return Ok(None);
    }
    let pin_parent = |path: &Path| -> io::Result<(File, PathBuf)> {
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?;
        let parent = opened_directory(&fs::canonicalize(parent)?)?;
        let pinned = fd_path(&parent).join(
            path.file_name()
                .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?,
        );
        Ok((parent, pinned))
    };
    // Keep both directory descriptors alive through rollback, even if a public parent is renamed.
    let (_input, source) = pin_parent(source)?;
    let (output, target) = pin_parent(target)?;
    if consent_source.dev() != output.metadata()?.dev() {
        return Ok(None);
    }
    let stage_root = fd_path(&output).join(format!(".winpoint-transfer-{}", nonce()));
    fs::DirBuilder::new().mode(0o700).create(&stage_root)?;
    let stage = opened_directory(&stage_root)?;
    exclusive_stage(&stage)?;
    let stage_metadata = stage.metadata()?;
    let new = fd_path(&stage).join("new");
    let old = fd_path(&stage).join("old");
    #[cfg(test)]
    if let Some(hook) = &mut progress.before_stage {
        hook();
    }
    let mut source_captured = false;
    let mut target_captured = false;
    let mut published = false;
    let mut source_metadata = None;
    let mut target_metadata = None;
    let mut commit = None;
    let result = (|| {
        loop {
            progress.checkpoint()?;
            if !fs::symlink_metadata(&source).is_ok_and(|m| unchanged(consent_source, &m)) {
                progress.uncertain_metrics();
                return Ok(Some(false));
            }
            if !fs::symlink_metadata(&target).is_ok_and(|m| unchanged(expected, &m)) {
                return Ok(Some(false));
            }
            commit = progress.begin_commit()?;
            if progress.control.as_ref().is_some_and(|job| job.pending()) {
                drop(commit.take());
                continue;
            }
            // begin_commit may have waited for Pause: consent must still describe both entries.
            if !fs::symlink_metadata(&source).is_ok_and(|m| unchanged(consent_source, &m)) {
                progress.uncertain_metrics();
                return Ok(Some(false));
            }
            if !fs::symlink_metadata(&target).is_ok_and(|m| unchanged(expected, &m)) {
                return Ok(Some(false));
            }
            break;
        }
        // Capture the source first. A raced foreign source is checked in private staging and
        // never published on the real target name. EXDEV here has changed neither public name.
        match capture_private(&source, &new, consent_source, names) {
            Ok(()) => source_captured = true,
            Err(error) if error.raw_os_error() == Some(libc::EXDEV) => return Ok(None),
            Err(error) => return Err(error),
        }
        let observed = fs::symlink_metadata(&new)?;
        let consent_matches = captured_matches(consent_source, &observed);
        source_metadata = Some(observed);
        if !consent_matches {
            progress.uncertain_metrics();
            return Ok(Some(false));
        }
        if !fs::symlink_metadata(&target).is_ok_and(|m| {
            if same_identity(consent_source, expected) {
                captured_matches(expected, &m) // Capturing one hard link also changes its sibling's ctime.
            } else {
                unchanged(expected, &m)
            }
        }) {
            return Ok(Some(false));
        }
        match capture_private(&target, &old, expected, names) {
            Ok(()) => target_captured = true,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Some(false)),
            Err(error) => return Err(error),
        }
        let observed = fs::symlink_metadata(&old)?;
        let consent_matches = captured_matches(expected, &observed);
        target_metadata = Some(observed);
        if !consent_matches {
            return Ok(Some(false));
        }
        if !fs::symlink_metadata(&new)
            .is_ok_and(|m| captured_matches(source_metadata.as_ref().unwrap(), &m))
        {
            return Err(io::Error::from_raw_os_error(libc::EBUSY));
        }
        let publication = publish_private(&new, &target, source_metadata.as_ref().unwrap(), names)?;
        source_captured = publication == Publication::Linked;
        published = true;
        record(&mut progress.overwritten_paths, &target);
        if !fs::symlink_metadata(&target).is_ok_and(|m| captured_matches(consent_source, &m)) {
            return Err(io::Error::from_raw_os_error(libc::EBUSY));
        }
        if source_captured && cleanup_alias(&new, source_metadata.as_ref().unwrap(), names).is_ok()
        {
            source_captured = false;
        }
        Ok(Some(true))
    })();
    let mut rollback_ok = true;
    if !published {
        if target_captured {
            rollback_ok &= target_metadata.as_ref().is_some_and(|owned| {
                restore_private(&old, &target, owned, &mut target_captured, names)
            });
        }
        if source_captured {
            rollback_ok &= source_metadata.as_ref().is_some_and(|owned| {
                restore_private(&new, &source, owned, &mut source_captured, names)
            });
        }
    } else if matches!(result, Ok(Some(true))) && target_captured {
        // A hard-linked source/target can acquire ctime from our later rename, so take a fresh
        // exact cleanup snapshot only after checking every consent-bound content/identity field.
        if let Ok(current) = fs::symlink_metadata(&old)
            && target_metadata
                .as_ref()
                .is_some_and(|owned| captured_matches(owned, &current))
            && cleanup_alias(&old, &current, names).is_ok()
        {
            target_captured = false;
        }
    }
    let incomplete = source_captured || target_captured;
    if source_captured {
        record(&mut progress.recovery_paths, &new);
    }
    if target_captured {
        record(&mut progress.recovery_paths, &old);
    }
    if !fs::symlink_metadata(&stage_root).is_ok_and(|m| same_identity(&stage_metadata, &m))
        || fs::remove_dir(&stage_root).is_err()
    {
        record(&mut progress.recovery_paths, &fd_path(&stage));
    }
    // No pause checkpoint or file-byte traversal runs inside this short namespace transaction.
    drop(commit);
    if matches!(result, Ok(Some(true))) {
        progress.add(consent_source.len());
        progress.complete_entries(Some(1));
    }
    if (incomplete || !rollback_ok) && !published {
        return Err(io::Error::from_raw_os_error(libc::EBUSY));
    }
    result
}

fn replace_copy(
    source: &Path,
    target: &Path,
    consent_source: &Metadata,
    expected: &Metadata,
    moving: bool,
    progress: &mut Progress<'_>,
) -> io::Result<bool> {
    replace_copy_using(
        source,
        target,
        consent_source,
        expected,
        moving,
        progress,
        &mut rename_noreplace,
    )
}

fn replace_copy_using(
    source: &Path,
    target: &Path,
    consent_source: &Metadata,
    expected: &Metadata,
    moving: bool,
    progress: &mut Progress<'_>,
    names: &mut impl TransferNames,
) -> io::Result<bool> {
    let parent = target
        .parent()
        .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?;
    let stage_root = parent.join(format!(".winpoint-transfer-{}", nonce()));
    fs::DirBuilder::new().mode(0o700).create(&stage_root)?;
    let stage = opened_directory(&stage_root)?;
    exclusive_stage(&stage)?;
    let new = fd_path(&stage).join("new");
    let old = fd_path(&stage).join("old");
    #[cfg(test)]
    if let Some(hook) = &mut progress.before_stage {
        hook();
    }
    let mark = progress.metric_mark();
    let mut snapshot = match copy_any(source, &new, progress, 0) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let _ = fs::remove_dir(&stage_root);
            return Err(error);
        }
    };
    let mut captured = false;
    let mut published = false;
    let mut new_alias = false;
    let mut old_metadata = None;
    let result = (|| {
        // copy_any may itself have paused before opening the source. Its snapshot must still be
        // the source shown in this exact question, rather than a new inode/content seen after resume.
        if !unchanged(consent_source, &snapshot.metadata) {
            return Ok(false);
        }
        let commit = loop {
            progress.checkpoint()?;
            verify_snapshot_checked(source, &snapshot, &mut || progress.checkpoint())?;
            verify_copied_checked(&new, &snapshot, &mut || progress.checkpoint())?;
            if !fs::symlink_metadata(target).is_ok_and(|m| unchanged(expected, &m)) {
                return Ok(false);
            }
            let commit = progress.begin_commit()?;
            verify_snapshot_checked(source, &snapshot, &mut || Ok(()))?;
            verify_copied_checked(&new, &snapshot, &mut || Ok(()))?;
            if progress.control.as_ref().is_some_and(|job| job.pending()) {
                drop(commit);
                continue;
            }
            if !fs::symlink_metadata(target).is_ok_and(|m| unchanged(expected, &m)) {
                return Ok(false);
            }
            break commit;
        };
        match capture_private(target, &old, expected, names) {
            Ok(()) => captured = true,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        }
        let observed = fs::symlink_metadata(&old)?;
        old_metadata = Some(observed.clone());
        if !captured_matches(expected, &observed) {
            return Ok(false);
        }
        let publication = publish_private(&new, target, &snapshot.copied, names)?;
        new_alias = publication == Publication::Linked;
        published = true;
        let published_metadata = fs::symlink_metadata(target)?;
        if !captured_matches(&snapshot.copied, &published_metadata) {
            return Err(io::Error::from_raw_os_error(libc::EBUSY));
        }
        snapshot.copied = published_metadata;
        if new_alias && cleanup_alias(&new, &snapshot.copied, names).is_ok() {
            new_alias = false;
        }
        // linkat and removal of its private alias change ctime. Source removal must validate
        // the final public inode snapshot, not the pre-link or pre-unlink staging metadata.
        let final_metadata = fs::symlink_metadata(target)?;
        if !captured_matches(&snapshot.copied, &final_metadata) {
            return Err(io::Error::from_raw_os_error(libc::EBUSY));
        }
        snapshot.copied = final_metadata;
        if moving {
            remove_source(source, target, &snapshot)?;
        }
        record(&mut progress.overwritten_paths, target);
        drop(commit);
        Ok(true)
    })();
    let mut rollback_ok = true;
    if !published {
        let changed = !unchanged(consent_source, &snapshot.metadata)
            || !fs::symlink_metadata(source).is_ok_and(|now| unchanged(consent_source, &now));
        progress.discard_copy(mark, &new, cleanup_snapshot(&new, &snapshot), changed);
        if captured {
            rollback_ok = old_metadata
                .as_ref()
                .is_some_and(|owned| restore_private(&old, target, owned, &mut captured, names));
        }
    }
    if captured {
        if matches!(result, Ok(true))
            && let Some(metadata) = &old_metadata
        {
            if cleanup_alias(&old, metadata, names).is_ok() {
                captured = false;
            }
        }
        if captured {
            record(&mut progress.recovery_paths, &old);
        }
    }
    if new_alias {
        record(&mut progress.recovery_paths, &new);
    }
    if fs::remove_dir(&stage_root).is_err() && lexists(&stage_root) {
        record(&mut progress.recovery_paths, &stage_root);
    }
    if !rollback_ok {
        return Err(io::Error::from_raw_os_error(libc::EBUSY));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Real rename/link/unlink operations in private temp directories, with only the unsupported
    // flag and exact syscall boundaries injected. No filesystem-specific host mount is required.
    #[derive(Default)]
    struct PortableNames {
        reserved: Option<Box<dyn FnMut(&Path)>>,
        capturing: Option<Box<dyn FnMut(&Path, &Path)>>,
        captured: Option<Box<dyn FnMut(&Path, &Path)>>,
        link_error: Option<i32>,
        remove_error: Option<&'static str>,
        removed: Vec<PathBuf>,
    }

    impl TransferNames for PortableNames {
        fn no_replace(&mut self, _source: &Path, _target: &Path) -> io::Result<()> {
            Err(io::Error::from_raw_os_error(libc::EINVAL))
        }
        fn capture_slot(&mut self, source: &Path, target: &Path) -> io::Result<()> {
            if let Some(callback) = &mut self.capturing {
                callback(source, target);
            }
            fs::rename(source, target)?;
            if let Some(callback) = &mut self.captured {
                callback(source, target);
            }
            Ok(())
        }
        fn link(&mut self, source: &Path, target: &Path) -> io::Result<()> {
            if source.file_name() == Some(OsStr::new("new"))
                && let Some(errno) = self.link_error.take()
            {
                return Err(io::Error::from_raw_os_error(errno));
            }
            let mut native = rename_noreplace;
            native.link(source, target)
        }
        fn remove_alias(&mut self, path: &Path) -> io::Result<()> {
            self.removed.push(display_path(path));
            if self
                .remove_error
                .is_some_and(|name| path.file_name() == Some(OsStr::new(name)))
            {
                return Err(io::Error::from_raw_os_error(libc::EACCES));
            }
            fs::remove_file(path)
        }
        fn reserved_slot(&mut self, path: &Path) -> io::Result<()> {
            if let Some(callback) = &mut self.reserved {
                callback(path);
            }
            Ok(())
        }
    }

    struct Fixture {
        root: PathBuf,
        service: Arc<crate::files::Files>,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("win11-conflicts-{}", nonce()));
            fs::create_dir(&root).unwrap();
            let service = crate::files::Files::for_tests(&root.join("service"));
            Self { root, service }
        }
        fn folder(&self, name: &str) -> PathBuf {
            let path = self.root.join(name);
            fs::create_dir_all(&path).unwrap();
            path
        }
        fn run(
            &self,
            sources: Vec<PathBuf>,
            dest: PathBuf,
            moving: bool,
            decide: impl Fn(&Value) -> (&'static str, bool) + Send + Sync + 'static,
        ) -> (Value, usize) {
            let controls = self.service.transfers.clone();
            let calls = Arc::new(AtomicUsize::new(0));
            let observed = calls.clone();
            let context = Context::new(
                self.root.clone(),
                self.root.join("Desktop"),
                "en".into(),
                false,
            );
            self.service.events.on("files.conflict",Arc::new(move|event| {
                observed.fetch_add(1,Ordering::SeqCst);
                let (choice,all)=decide(event);
                if choice=="controlCancel" {
                    controls.call(&json!({"jobId":event["jobId"],"action":"cancel"}),&context,false).unwrap();
                } else {
                    controls.resolve(&json!({"jobId":event["jobId"],"conflictId":event["conflictId"],"choice":choice,"applyToAll":all}),&context,false).unwrap();
                }
            }));
            let mut context = self.service.context();
            context.language = "en".into();
            let result = transfer_with_options(
                sources,
                dest,
                moving,
                Some("interactive-test"),
                true,
                &context,
            )
            .unwrap();
            (result, calls.load(Ordering::SeqCst))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.service.events.shutdown();
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn replacement_is_explicit_and_old_legacy_unique_naming_stays_compatible() {
        let f = Fixture::new();
        let src = f.folder("src");
        let dest = f.folder("dest");
        fs::write(src.join("file"), b"new contents").unwrap();
        fs::write(dest.join("file"), b"old contents").unwrap();
        let (result, calls) = f.run(vec![src.join("file")], dest.clone(), false, |event| {
            assert_eq!(event["kind"], "collision");
            assert_eq!(event["source"]["size"], 12);
            ("replace", false)
        });
        assert_eq!(calls, 1);
        assert_eq!(result["ok"], true);
        assert_eq!(result["undoable"], false);
        assert_eq!(result["overwrittenPaths"], json!([dest.join("file")]));
        assert_eq!(fs::read(dest.join("file")).unwrap(), b"new contents");
        assert_eq!(fs::read(src.join("file")).unwrap(), b"new contents");
        assert!(result["recoveryPaths"].as_array().unwrap().is_empty());
        let context = Context::new(f.root.clone(), f.root.join("Desktop"), "en".into(), false);
        let legacy = transfer(
            vec![src.join("file")],
            dest.clone(),
            false,
            Some("legacy"),
            &context,
        )
        .unwrap();
        assert_eq!(legacy["ok"], true);
        assert!(dest.join("file (2)").is_file());
    }

    #[test]
    fn skip_and_keep_both_apply_to_later_conflicts_without_overwriting() {
        for choice in ["skip", "keepBoth"] {
            let f = Fixture::new();
            let src = f.folder("src");
            let dest = f.folder("dest");
            for name in ["a", "b"] {
                fs::write(src.join(name), b"new").unwrap();
                fs::write(dest.join(name), b"existing").unwrap();
            }
            let (result, calls) = f.run(
                vec![src.join("a"), src.join("b")],
                dest.clone(),
                false,
                move |_| (choice, true),
            );
            assert_eq!(calls, 1);
            assert_eq!(result["ok"], true);
            for name in ["a", "b"] {
                assert_eq!(fs::read(dest.join(name)).unwrap(), b"existing");
                assert!(src.join(name).exists());
            }
            if choice == "skip" {
                assert_eq!(result["skipped"].as_array().unwrap().len(), 2);
            } else {
                assert_eq!(fs::read(dest.join("a (2)")).unwrap(), b"new");
                assert!(dest.join("b (2)").exists());
            }
        }
    }

    #[test]
    fn changed_target_after_answer_requires_a_fresh_question_even_with_apply_all() {
        let f = Fixture::new();
        let src = f.folder("src");
        let dest = f.folder("dest");
        let target = dest.join("file");
        fs::write(src.join("file"), b"source").unwrap();
        fs::write(&target, b"old").unwrap();
        let attempts = AtomicUsize::new(0);
        let changed = target.clone();
        let (result, calls) = f.run(vec![src.join("file")], dest, false, move |event| {
            assert_eq!(event["kind"], "collision");
            if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                fs::write(&changed, b"foreign changed target").unwrap();
                ("replace", true)
            } else {
                ("skip", false)
            }
        });
        assert_eq!(calls, 2);
        assert_eq!(result["ok"], true);
        assert_eq!(fs::read(target).unwrap(), b"foreign changed target");
        assert!(src.join("file").exists());
    }

    #[test]
    fn replacing_a_target_link_never_writes_to_its_referent() {
        let f = Fixture::new();
        let src = f.folder("src");
        let dest = f.folder("dest");
        let outside = f.root.join("outside");
        fs::write(&outside, b"outside stays").unwrap();
        fs::write(src.join("file"), b"new").unwrap();
        symlink(&outside, dest.join("file")).unwrap();
        let (result, _) = f.run(vec![src.join("file")], dest.clone(), false, |event| {
            assert_eq!(event["target"]["kind"], "link");
            ("replace", false)
        });
        assert_eq!(result["ok"], true);
        assert_eq!(fs::read(&outside).unwrap(), b"outside stays");
        assert!(!dest.join("file").is_symlink());
        assert_eq!(fs::read(dest.join("file")).unwrap(), b"new");
    }

    #[test]
    fn merge_preserves_unrelated_children_and_replaces_only_the_answered_file() {
        let f = Fixture::new();
        let src = f.folder("src/tree");
        let dest = f.folder("dest");
        let existing = f.folder("dest/tree");
        fs::write(src.join("conflict"), b"new").unwrap();
        fs::write(src.join("added"), b"added").unwrap();
        fs::write(existing.join("conflict"), b"old").unwrap();
        fs::write(existing.join("untouched"), b"foreign").unwrap();
        let (result, calls) = f.run(vec![src.clone()], dest, false, |event| {
            assert_eq!(event["target"]["name"], "conflict");
            ("replace", false)
        });
        assert_eq!(calls, 1);
        assert_eq!(result["ok"], true);
        assert_eq!(result["undoable"], false);
        assert_eq!(fs::read(existing.join("untouched")).unwrap(), b"foreign");
        assert_eq!(fs::read(existing.join("conflict")).unwrap(), b"new");
        assert_eq!(fs::read(existing.join("added")).unwrap(), b"added");
        assert!(src.join("conflict").exists());
        assert_eq!(result["mergedPaths"], json!([existing]));
    }

    #[test]
    fn moving_merge_skip_keeps_remaining_source_and_cancel_wakes_the_question() {
        for choice in ["skip", "controlCancel"] {
            let f = Fixture::new();
            let src = f.folder("src/tree");
            let dest = f.folder("dest");
            let existing = f.folder("dest/tree");
            fs::write(src.join("conflict"), b"source").unwrap();
            fs::write(existing.join("conflict"), b"target").unwrap();
            fs::write(existing.join("foreign"), b"keep").unwrap();
            let (result, calls) = f.run(vec![src.clone()], dest, true, move |_| (choice, false));
            assert_eq!(calls, 1);
            assert_eq!(result["cancelled"], choice == "controlCancel");
            assert_eq!(fs::read(src.join("conflict")).unwrap(), b"source");
            assert_eq!(fs::read(existing.join("conflict")).unwrap(), b"target");
            assert_eq!(fs::read(existing.join("foreign")).unwrap(), b"keep");
            assert_eq!(result["undoable"], false);
        }
    }

    #[test]
    fn same_folder_copy_through_a_directory_link_uses_copy_name_without_question() {
        let f = Fixture::new();
        let src = f.folder("src");
        fs::write(src.join("file.txt"), b"source").unwrap();
        let alias = f.root.join("alias");
        symlink(&src, &alias).unwrap();
        let (result, calls) = f.run(vec![alias.join("file.txt")], src.clone(), false, |_| {
            panic!("same-folder copy must not ask")
        });
        assert_eq!(calls, 0);
        assert_eq!(result["ok"], true);
        assert!(src.join("file - Copy.txt").exists());
        assert_eq!(fs::read(src.join("file.txt")).unwrap(), b"source");
    }

    #[test]
    fn permission_error_retry_waits_for_user_action_then_retries_the_safe_copy() {
        // The production harness runs as the ordinary desktop uid; root bypasses this permission boundary.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new();
        let src = f.folder("src");
        let dest = f.folder("dest");
        let source = src.join("file");
        fs::write(&source, b"source").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0)).unwrap();
        let readable = source.clone();
        let (result, calls) = f.run(vec![source.clone()], dest.clone(), false, move |event| {
            assert_eq!(event["kind"], "error");
            assert_eq!(event["retrySafe"], true);
            assert!(
                event["choices"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("retry"))
            );
            fs::set_permissions(&readable, fs::Permissions::from_mode(0o600)).unwrap();
            ("retry", false)
        });
        assert_eq!(calls, 1);
        assert_eq!(result["ok"], true);
        assert_eq!(fs::read(dest.join("file")).unwrap(), b"source");
    }

    #[test]
    fn source_changed_during_pre_staging_pause_invalidates_replace_consent() {
        let f = Fixture::new();
        let src = f.folder("src");
        let dest = f.folder("dest");
        let source = src.join("file");
        let target = dest.join("file");
        fs::write(&source, b"source shown in question").unwrap();
        fs::write(&target, b"old target must remain").unwrap();
        let source_consent = fs::symlink_metadata(&source).unwrap();
        let target_consent = fs::symlink_metadata(&target).unwrap();
        let context = Context::new(f.root.clone(), f.root.join("Desktop"), "en".into(), false);
        let job = context
            .transfers
            .register("source-consent", &context)
            .unwrap();
        let gate = job.clone();
        let worker_source = source.clone();
        let worker_target = target.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let mut progress = super::super::tests::progress();
            progress.control = Some(gate.clone());
            progress.total = Some(source_consent.len());
            progress.entry_count = Some(1);
            progress.before_stage = Some(Box::new(move || {
                gate.request("pause");
                tx.send(()).unwrap();
            }));
            let result = replace(
                &worker_source,
                &worker_target,
                &source_consent,
                &target_consent,
                false,
                &mut progress,
            );
            (result, progress.total, progress.entry_count)
        });
        rx.recv_timeout(Duration::from_secs(3)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while job.json()["state"] != "paused" && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(job.json()["state"], "paused");
        fs::remove_file(&source).unwrap();
        fs::write(&source, b"new source after question").unwrap();
        job.request("resume");
        let (result, total, count) = worker.join().unwrap();
        assert!(
            !result.unwrap(),
            "false causes a new question rather than replacing"
        );
        assert_eq!(
            total, None,
            "changed source invalidates the old byte estimate"
        );
        assert_eq!(
            count, None,
            "changed source invalidates the old entry estimate"
        );
        assert_eq!(fs::read(source).unwrap(), b"new source after question");
        assert_eq!(fs::read(&target).unwrap(), b"old target must remain");
        assert_eq!(
            fs::read_dir(dest).unwrap().count(),
            1,
            "owned staging was cleaned"
        );
    }

    #[test]
    fn discarded_stage_and_new_consent_count_bytes_once_and_keep_completed_sibling_metrics() {
        use std::sync::atomic::AtomicBool;
        let f = Fixture::new();
        let src = f.folder("src");
        let dest = f.folder("dest");
        let sibling = vec![0x31; 41];
        let contents = vec![0x43; 2 * 1024 * 1024];
        fs::write(src.join("sibling"), &sibling).unwrap();
        fs::write(src.join("file"), &contents).unwrap();
        let target = dest.join("file");
        fs::write(&target, b"old").unwrap();
        let mut context = f.service.context();
        context.language = "en".into();
        let job = context
            .transfers
            .register("stage-metrics", &context)
            .unwrap();
        let questions = Arc::new(AtomicUsize::new(0));
        let observed = questions.clone();
        let controls = context.transfers.clone();
        let resolver = Context::new(f.root.clone(), f.root.join("Desktop"), "en".into(), false);
        f.service.events.on("files.conflict",Arc::new(move|event| {
            assert_eq!(event["kind"],"collision");observed.fetch_add(1,Ordering::SeqCst);
            controls.resolve(&json!({"jobId":event["jobId"],"conflictId":event["conflictId"],"choice":"replace"}),&resolver,false).unwrap();
        }));
        let samples = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = samples.clone();
        let discard_started = Arc::new(AtomicBool::new(false));
        let after_discard = discard_started.clone();
        f.service.events.on(
            "files.progress",
            Arc::new(move |event| {
                recorded.lock().unwrap().push((
                    event["done"].as_u64().unwrap(),
                    after_discard.load(Ordering::SeqCst),
                ))
            }),
        );
        let mut progress = super::super::tests::progress();
        progress.ctx = Some(&context);
        progress.control = Some(job);
        progress.total = Some((sibling.len() + contents.len()) as u64);
        progress.entry_count = Some(2);
        copy_any(
            &src.join("sibling"),
            &dest.join("sibling"),
            &mut progress,
            0,
        )
        .unwrap();
        let changed = target.clone();
        let first = AtomicBool::new(true);
        progress.before_commit = Some(Box::new(move || {
            if first.swap(false, Ordering::SeqCst) {
                fs::write(&changed, b"target changed after staging").unwrap();
                discard_started.store(true, Ordering::SeqCst);
            }
        }));
        entry(&src.join("file"), &target, false, &mut progress, 0).unwrap();
        assert_eq!(
            questions.load(Ordering::SeqCst),
            2,
            "discarding consent requires a new real question"
        );
        assert_eq!(progress.done, (sibling.len() + contents.len()) as u64);
        assert_eq!(progress.total, Some(progress.done));
        assert_eq!(progress.entries_done, Some(2));
        assert_eq!(fs::read(dest.join("sibling")).unwrap(), sibling);
        assert_eq!(fs::read(target).unwrap(), contents);
        assert!(
            samples
                .lock()
                .unwrap()
                .iter()
                .any(|(done, after_discard)| *after_discard && *done == sibling.len() as u64),
            "a reset is published before retry"
        );
    }

    #[test]
    fn mid_copy_error_cleanup_and_explicit_retry_reset_only_that_attempt() {
        let f = Fixture::new();
        let src = f.folder("src");
        let dest = f.folder("dest");
        let sibling = vec![0x31; 41];
        let contents = vec![0x44; 12 * 1024 * 1024];
        fs::write(src.join("sibling"), &sibling).unwrap();
        fs::write(src.join("file"), &contents).unwrap();
        let mut context = f.service.context();
        context.language = "en".into();
        let job = context
            .transfers
            .register("retry-metrics", &context)
            .unwrap();
        let questions = Arc::new(AtomicUsize::new(0));
        let observed = questions.clone();
        let controls = context.transfers.clone();
        let resolver = Context::new(f.root.clone(), f.root.join("Desktop"), "en".into(), false);
        f.service.events.on("files.conflict",Arc::new(move|event| {
            assert_eq!(event["kind"],"error");assert_eq!(event["retrySafe"],true);observed.fetch_add(1,Ordering::SeqCst);
            controls.resolve(&json!({"jobId":event["jobId"],"conflictId":event["conflictId"],"choice":"retry"}),&resolver,false).unwrap();
        }));
        let mut progress = super::super::tests::progress();
        progress.ctx = Some(&context);
        progress.control = Some(job);
        progress.total = Some((sibling.len() + contents.len()) as u64);
        progress.entry_count = Some(2);
        copy_any(
            &src.join("sibling"),
            &dest.join("sibling"),
            &mut progress,
            0,
        )
        .unwrap();
        // Inject ENOSPC at the first real progress chunk above one MiB, before this larger copy finishes.
        progress.fail_after_write = Some(1024 * 1024);
        entry(
            &src.join("file"),
            &dest.join("file"),
            false,
            &mut progress,
            0,
        )
        .unwrap();
        assert_eq!(questions.load(Ordering::SeqCst), 1);
        assert_eq!(progress.done, (sibling.len() + contents.len()) as u64);
        assert_eq!(progress.total, Some(progress.done));
        assert_eq!(progress.entry_count, Some(2));
        assert_eq!(progress.entries_done, Some(2));
        assert!(progress.partial_paths.is_empty());
        assert_eq!(fs::read(dest.join("sibling")).unwrap(), sibling);
        assert_eq!(fs::read(dest.join("file")).unwrap(), contents);
        assert_eq!(fs::read(src.join("file")).unwrap(), contents);
    }

    #[test]
    fn overwrite_move_preserves_the_source_inode_without_reading_file_bytes() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new();
        let src = f.folder("src");
        let dest = f.folder("dest");
        let source = src.join("file");
        let target = dest.join("file");
        fs::write(&source, b"move this inode").unwrap();
        fs::write(&target, b"old destination").unwrap();
        // Rename requires parent-directory access, not read access to the file being moved.
        fs::set_permissions(&source, fs::Permissions::from_mode(0)).unwrap();
        let original = fs::symlink_metadata(&source).unwrap();
        let (result, calls) = f.run(vec![source.clone()], dest.clone(), true, |_| {
            ("replace", false)
        });
        assert_eq!(calls, 1);
        assert_eq!(result["ok"], true);
        assert_eq!(result["undoable"], false);
        assert_eq!(result["overwrittenPaths"], json!([target]));
        assert!(result["recoveryPaths"].as_array().unwrap().is_empty());
        assert!(!lexists(&source));
        assert!(captured_matches(
            &original,
            &fs::symlink_metadata(&target).unwrap()
        ));
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(fs::read(target).unwrap(), b"move this inode");
        assert_eq!(fs::read_dir(dest).unwrap().count(), 1);
    }

    #[test]
    fn overwrite_move_keeps_symlinks_as_links_and_does_not_touch_either_referent() {
        let f = Fixture::new();
        let src = f.folder("src");
        let dest = f.folder("dest");
        let source_referent = f.root.join("source-referent");
        let target_referent = f.root.join("target-referent");
        fs::write(&source_referent, b"source referent remains").unwrap();
        fs::write(&target_referent, b"target referent remains").unwrap();
        let source = src.join("link");
        let target = dest.join("link");
        symlink(&source_referent, &source).unwrap();
        symlink(&target_referent, &target).unwrap();
        let original = fs::symlink_metadata(&source).unwrap();
        let (result, calls) = f.run(vec![source.clone()], dest, true, |_| ("replace", false));
        assert_eq!(calls, 1);
        assert_eq!(result["ok"], true);
        assert!(!lexists(&source));
        assert!(captured_matches(
            &original,
            &fs::symlink_metadata(&target).unwrap()
        ));
        assert_eq!(fs::read_link(&target).unwrap(), source_referent);
        assert_eq!(
            fs::read(source_referent).unwrap(),
            b"source referent remains"
        );
        assert_eq!(
            fs::read(target_referent).unwrap(),
            b"target referent remains"
        );
    }

    #[test]
    fn overwrite_move_adds_metrics_once_and_removes_an_old_hard_link_safely() {
        let f = Fixture::new();
        let src = f.folder("src");
        let dest = f.folder("dest");
        let source = src.join("file");
        let target = dest.join("file");
        fs::write(&source, b"same hard-linked inode").unwrap();
        fs::hard_link(&source, &target).unwrap();
        let consent = fs::symlink_metadata(&source).unwrap();
        let expected = fs::symlink_metadata(&target).unwrap();
        let mut progress = super::super::tests::progress();
        progress.done = 41;
        progress.entries_done = Some(1);
        assert!(replace(&source, &target, &consent, &expected, true, &mut progress).unwrap());
        assert_eq!(progress.done, 41 + consent.len());
        assert_eq!(progress.entries_done, Some(2));
        assert_eq!(progress.overwritten_paths, vec![target.clone()]);
        assert!(progress.recovery_paths.is_empty());
        assert!(same_identity(
            &consent,
            &fs::symlink_metadata(target).unwrap()
        ));
        assert!(!lexists(&source));
        assert_eq!(fs::read_dir(dest).unwrap().count(), 1);
    }

    #[test]
    fn overwrite_move_rejects_a_source_replaced_between_consent_and_capture() {
        let f = Fixture::new();
        let src = f.folder("src");
        let dest = f.folder("dest");
        let source = src.join("file");
        let retained = src.join("retained-original");
        let target = dest.join("file");
        fs::write(&source, b"consented source").unwrap();
        fs::write(&target, b"existing target").unwrap();
        let consent = fs::symlink_metadata(&source).unwrap();
        let expected = fs::symlink_metadata(&target).unwrap();
        let mut progress = super::super::tests::progress();
        let mut first = true;
        let result = replace_move_with(
            &source,
            &target,
            &consent,
            &expected,
            &mut progress,
            |from, to| {
                if first {
                    first = false;
                    fs::rename(&source, &retained)?;
                    fs::write(&source, b"raced source must not be published")?;
                }
                rename_noreplace(from, to)
            },
        )
        .unwrap();
        assert_eq!(result, Some(false));
        assert_eq!(
            fs::read(source).unwrap(),
            b"raced source must not be published"
        );
        assert_eq!(fs::read(retained).unwrap(), b"consented source");
        assert_eq!(fs::read(target).unwrap(), b"existing target");
        assert_eq!(progress.done, 0);
        assert_eq!(progress.total, None);
        assert!(progress.overwritten_paths.is_empty());
        assert!(progress.recovery_paths.is_empty());
    }

    #[test]
    fn overwrite_move_restores_source_when_the_consented_target_changes_after_capture() {
        let f = Fixture::new();
        let src = f.folder("src");
        let dest = f.folder("dest");
        let source = src.join("file");
        let target = dest.join("file");
        fs::write(&source, b"source stays available").unwrap();
        fs::write(&target, b"old target").unwrap();
        let consent = fs::symlink_metadata(&source).unwrap();
        let expected = fs::symlink_metadata(&target).unwrap();
        let mut progress = super::super::tests::progress();
        let mut first = true;
        let result = replace_move_with(
            &source,
            &target,
            &consent,
            &expected,
            &mut progress,
            |from, to| {
                rename_noreplace(from, to)?;
                if first {
                    first = false;
                    fs::write(&target, b"changed target must not be overwritten")?;
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(result, Some(false));
        assert!(captured_matches(
            &consent,
            &fs::symlink_metadata(&source).unwrap()
        ));
        assert_eq!(fs::read(source).unwrap(), b"source stays available");
        assert_eq!(
            fs::read(target).unwrap(),
            b"changed target must not be overwritten"
        );
        assert_eq!(progress.done, 0);
        assert!(progress.recovery_paths.is_empty());
        assert_eq!(fs::read_dir(dest).unwrap().count(), 1);
    }

    #[test]
    fn overwrite_move_never_overwrites_a_new_target_and_keeps_the_old_target_as_recovery() {
        let f = Fixture::new();
        let src = f.folder("src");
        let dest = f.folder("dest");
        let source = src.join("file");
        let target = dest.join("file");
        fs::write(&source, b"source restored").unwrap();
        fs::write(&target, b"old target recovered").unwrap();
        let consent = fs::symlink_metadata(&source).unwrap();
        let expected = fs::symlink_metadata(&target).unwrap();
        let mut progress = super::super::tests::progress();
        let result = replace_move_with(
            &source,
            &target,
            &consent,
            &expected,
            &mut progress,
            |from, to| {
                rename_noreplace(from, to)?;
                if to.file_name() == Some(OsStr::new("old")) {
                    fs::write(&target, b"new foreign target")?;
                }
                Ok(())
            },
        );
        assert_eq!(result.unwrap_err().raw_os_error(), Some(libc::EBUSY));
        assert_eq!(fs::read(source).unwrap(), b"source restored");
        assert_eq!(fs::read(target).unwrap(), b"new foreign target");
        let old = progress
            .recovery_paths
            .iter()
            .find(|p| p.file_name() == Some(OsStr::new("old")))
            .unwrap();
        assert_eq!(fs::read(old).unwrap(), b"old target recovered");
        assert!(
            progress
                .recovery_paths
                .contains(&old.parent().unwrap().to_path_buf())
        );
        assert_eq!(progress.done, 0);
        assert!(progress.overwritten_paths.is_empty());
    }

    #[test]
    fn overwrite_move_never_replaces_a_foreign_source_created_during_rollback() {
        let f = Fixture::new();
        let src = f.folder("src");
        let dest = f.folder("dest");
        let source = src.join("file");
        let target = dest.join("file");
        fs::write(&source, b"captured source recovered").unwrap();
        fs::write(&target, b"untouched target").unwrap();
        let consent = fs::symlink_metadata(&source).unwrap();
        let expected = fs::symlink_metadata(&target).unwrap();
        let mut progress = super::super::tests::progress();
        let result = replace_move_with(
            &source,
            &target,
            &consent,
            &expected,
            &mut progress,
            |from, to| {
                if to.file_name() == Some(OsStr::new("old")) {
                    fs::write(&source, b"foreign new source")?;
                    return Err(io::Error::from_raw_os_error(libc::EIO));
                }
                rename_noreplace(from, to)
            },
        );
        assert_eq!(result.unwrap_err().raw_os_error(), Some(libc::EBUSY));
        assert_eq!(fs::read(source).unwrap(), b"foreign new source");
        assert_eq!(fs::read(target).unwrap(), b"untouched target");
        let recovered = progress
            .recovery_paths
            .iter()
            .find(|p| p.file_name() == Some(OsStr::new("new")))
            .unwrap();
        assert_eq!(fs::read(recovered).unwrap(), b"captured source recovered");
        assert!(same_identity(
            &consent,
            &fs::symlink_metadata(recovered).unwrap()
        ));
        assert_eq!(progress.done, 0);
    }

    #[test]
    fn overwrite_move_rollback_uses_pinned_parents_after_both_public_directories_are_replaced() {
        let f = Fixture::new();
        let src = f.folder("src");
        let dest = f.folder("dest");
        let moved_src = f.root.join("original-src");
        let moved_dest = f.root.join("original-dest");
        let source = src.join("file");
        let target = dest.join("file");
        fs::write(&source, b"original source").unwrap();
        fs::write(&target, b"original target").unwrap();
        let consent = fs::symlink_metadata(&source).unwrap();
        let expected = fs::symlink_metadata(&target).unwrap();
        let mut progress = super::super::tests::progress();
        let mut first = true;
        let result = replace_move_with(
            &source,
            &target,
            &consent,
            &expected,
            &mut progress,
            |from, to| {
                if first {
                    first = false;
                    rename_noreplace(from, to)?;
                    fs::rename(&src, &moved_src)?;
                    fs::create_dir(&src)?;
                    fs::write(&source, b"foreign source directory")?;
                    return Ok(());
                }
                if to.file_name() == Some(OsStr::new("old")) {
                    fs::rename(&dest, &moved_dest)?;
                    fs::create_dir(&dest)?;
                    fs::write(&target, b"foreign target directory")?;
                    return Err(io::Error::from_raw_os_error(libc::EIO));
                }
                rename_noreplace(from, to)
            },
        );
        assert_eq!(result.unwrap_err().raw_os_error(), Some(libc::EIO));
        assert_eq!(fs::read(source).unwrap(), b"foreign source directory");
        assert_eq!(fs::read(target).unwrap(), b"foreign target directory");
        assert_eq!(
            fs::read(moved_src.join("file")).unwrap(),
            b"original source"
        );
        assert_eq!(
            fs::read(moved_dest.join("file")).unwrap(),
            b"original target"
        );
        assert_eq!(fs::read_dir(moved_dest).unwrap().count(), 1);
        assert!(progress.recovery_paths.is_empty());
    }

    #[test]
    fn overwrite_move_exdev_keeps_both_public_names_intact_before_existing_copy_fallback() {
        let f = Fixture::new();
        let src = f.folder("src");
        let dest = f.folder("dest");
        let source = src.join("file");
        let target = dest.join("file");
        fs::write(&source, b"cross-device source").unwrap();
        fs::write(&target, b"old target").unwrap();
        let consent = fs::symlink_metadata(&source).unwrap();
        let expected = fs::symlink_metadata(&target).unwrap();
        let mut progress = super::super::tests::progress();
        let mut calls = 0;
        let result = replace_move_with(
            &source,
            &target,
            &consent,
            &expected,
            &mut progress,
            |_, _| {
                calls += 1;
                Err(io::Error::from_raw_os_error(libc::EXDEV))
            },
        )
        .unwrap();
        assert_eq!(result, None);
        assert_eq!(calls, 1);
        assert!(unchanged(&consent, &fs::symlink_metadata(&source).unwrap()));
        assert!(unchanged(
            &expected,
            &fs::symlink_metadata(&target).unwrap()
        ));
        assert_eq!(progress.done, 0);
        assert!(progress.recovery_paths.is_empty());
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 1);
        assert!(replace_copy(&source, &target, &consent, &expected, true, &mut progress).unwrap());
        assert!(!lexists(&source));
        assert_eq!(fs::read(&target).unwrap(), b"cross-device source");
        assert!(!same_identity(
            &consent,
            &fs::symlink_metadata(target).unwrap()
        ));
        assert_eq!(progress.done, consent.len());
        assert!(progress.recovery_paths.is_empty());
    }

    #[test]
    fn overwrite_move_cancellation_before_commit_does_not_capture_either_entry() {
        let f = Fixture::new();
        let src = f.folder("src");
        let dest = f.folder("dest");
        let source = src.join("file");
        let target = dest.join("file");
        fs::write(&source, b"source").unwrap();
        fs::write(&target, b"target").unwrap();
        let consent = fs::symlink_metadata(&source).unwrap();
        let expected = fs::symlink_metadata(&target).unwrap();
        let context = Context::new(f.root.clone(), f.root.join("Desktop"), "en".into(), false);
        let job = context
            .transfers
            .register("cancel-overwrite-rename", &context)
            .unwrap();
        let cancel = job.clone();
        let mut progress = super::super::tests::progress();
        progress.control = Some(job);
        progress.before_commit = Some(Box::new(move || cancel.request("cancel")));
        let result = replace_move_with(
            &source,
            &target,
            &consent,
            &expected,
            &mut progress,
            |_, _| panic!("cancelled before the first rename"),
        );
        assert!(control::is_cancelled(&result.unwrap_err()));
        assert!(unchanged(&consent, &fs::symlink_metadata(source).unwrap()));
        assert!(unchanged(&expected, &fs::symlink_metadata(target).unwrap()));
        assert!(progress.recovery_paths.is_empty());
        assert_eq!(fs::read_dir(dest).unwrap().count(), 1);
    }

    #[test]
    fn overwrite_move_pauses_before_capture_and_resumes_without_changing_consent() {
        let f = Fixture::new();
        let src = f.folder("src");
        let dest = f.folder("dest");
        let source = src.join("file");
        let target = dest.join("file");
        fs::write(&source, b"source").unwrap();
        fs::write(&target, b"target").unwrap();
        let consent = fs::symlink_metadata(&source).unwrap();
        let expected = fs::symlink_metadata(&target).unwrap();
        let context = Context::new(f.root.clone(), f.root.join("Desktop"), "en".into(), false);
        let job = context
            .transfers
            .register("pause-overwrite-rename", &context)
            .unwrap();
        let pause = job.clone();
        let worker_source = source.clone();
        let worker_target = target.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let mut progress = super::super::tests::progress();
            progress.control = Some(pause.clone());
            progress.before_stage = Some(Box::new(move || {
                pause.request("pause");
                tx.send(()).unwrap();
            }));
            replace(
                &worker_source,
                &worker_target,
                &consent,
                &expected,
                true,
                &mut progress,
            )
        });
        rx.recv_timeout(Duration::from_secs(3)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while job.json()["state"] != "paused" && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(job.json()["state"], "paused");
        assert_eq!(fs::read(&source).unwrap(), b"source");
        assert_eq!(fs::read(&target).unwrap(), b"target");
        job.request("resume");
        assert!(worker.join().unwrap().unwrap());
        assert!(!lexists(&source));
        assert_eq!(fs::read(target).unwrap(), b"source");
    }

    #[test]
    fn portable_overwrite_move_preserves_regular_and_dangling_link_inodes_without_public_unlink() {
        for is_link in [false, true] {
            let f = Fixture::new();
            let src = f.folder("src");
            let dest = f.folder("dest");
            let source = src.join("file");
            let target = dest.join("file");
            if is_link {
                symlink("missing-referent", &source).unwrap();
            } else {
                fs::write(&source, b"same inode, no copy").unwrap();
            }
            fs::write(&target, b"old destination").unwrap();
            let consent = fs::symlink_metadata(&source).unwrap();
            let expected = fs::symlink_metadata(&target).unwrap();
            let mut progress = super::super::tests::progress();
            progress.done = 41;
            progress.entries_done = Some(1);
            let mut names = PortableNames::default();
            assert_eq!(
                replace_move_using(
                    &source,
                    &target,
                    &consent,
                    &expected,
                    &mut progress,
                    &mut names
                )
                .unwrap(),
                Some(true)
            );
            assert!(!lexists(&source));
            assert!(captured_matches(
                &consent,
                &fs::symlink_metadata(&target).unwrap()
            ));
            if is_link {
                assert_eq!(
                    fs::read_link(&target).unwrap(),
                    PathBuf::from("missing-referent")
                );
            } else {
                assert_eq!(fs::read(&target).unwrap(), b"same inode, no copy");
            }
            assert_eq!(progress.done, 41 + consent.len());
            assert_eq!(progress.entries_done, Some(2));
            assert_eq!(progress.overwritten_paths, vec![target.clone()]);
            assert!(progress.recovery_paths.is_empty());
            assert!(
                names
                    .removed
                    .iter()
                    .all(|path| path != &source && path != &target)
            );
            assert_eq!(fs::read_dir(dest).unwrap().count(), 1);
        }
    }

    #[test]
    fn portable_copy_publication_refreshes_snapshot_after_link_cleanup_before_deleting_source() {
        for moving in [false, true] {
            let f = Fixture::new();
            let src = f.folder("src");
            let dest = f.folder("dest");
            let source = src.join("file");
            let target = dest.join("file");
            let contents = vec![0x58; 4097];
            fs::write(&source, &contents).unwrap();
            fs::write(&target, b"old destination").unwrap();
            let consent = fs::symlink_metadata(&source).unwrap();
            let expected = fs::symlink_metadata(&target).unwrap();
            let mut progress = super::super::tests::progress();
            let mut names = PortableNames::default();
            assert!(
                replace_copy_using(
                    &source,
                    &target,
                    &consent,
                    &expected,
                    moving,
                    &mut progress,
                    &mut names
                )
                .unwrap()
            );
            assert_eq!(fs::read(&target).unwrap(), contents);
            assert!(!same_identity(
                &consent,
                &fs::symlink_metadata(&target).unwrap()
            ));
            assert_eq!(lexists(&source), !moving);
            if !moving {
                assert!(unchanged(&consent, &fs::symlink_metadata(&source).unwrap()));
            }
            assert_eq!(progress.done, contents.len() as u64);
            assert_eq!(progress.entries_done, Some(1));
            assert!(progress.recovery_paths.is_empty());
            assert_eq!(fs::read_dir(dest).unwrap().count(), 1);
        }
    }

    #[test]
    fn portable_capture_never_replaces_or_unlinks_a_foreign_reserved_slot() {
        let f = Fixture::new();
        let src = f.folder("src");
        let dest = f.folder("dest");
        let source = src.join("file");
        let target = dest.join("file");
        fs::write(&source, b"source untouched").unwrap();
        fs::write(&target, b"target untouched").unwrap();
        let consent = fs::symlink_metadata(&source).unwrap();
        let expected = fs::symlink_metadata(&target).unwrap();
        let mut names = PortableNames {
            reserved: Some(Box::new(|slot| {
                fs::remove_file(slot).unwrap();
                fs::write(slot, b"foreign reserved slot").unwrap();
            })),
            ..Default::default()
        };
        let mut progress = super::super::tests::progress();
        let error = replace_move_using(
            &source,
            &target,
            &consent,
            &expected,
            &mut progress,
            &mut names,
        )
        .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::EBUSY));
        assert!(unchanged(&consent, &fs::symlink_metadata(&source).unwrap()));
        assert!(unchanged(
            &expected,
            &fs::symlink_metadata(&target).unwrap()
        ));
        assert!(
            names.removed.is_empty(),
            "foreign reserve is not our cleanup target"
        );
        let stage = progress
            .recovery_paths
            .iter()
            .find(|path| path.is_dir())
            .unwrap();
        assert_eq!(
            fs::read(stage.join("new")).unwrap(),
            b"foreign reserved slot"
        );
        assert_eq!(progress.done, 0);
    }

    #[test]
    fn portable_capture_checks_a_raced_public_source_before_and_after_the_private_rename() {
        for after_guard in [false, true] {
            let f = Fixture::new();
            let src = f.folder("src");
            let dest = f.folder("dest");
            let source = src.join("file");
            let retained = src.join("consented-original");
            let target = dest.join("file");
            fs::write(&source, b"consented source").unwrap();
            fs::write(&target, b"old target").unwrap();
            let consent = fs::symlink_metadata(&source).unwrap();
            let expected = fs::symlink_metadata(&target).unwrap();
            let changed_source = source.clone();
            let mut changed = false;
            let mut race = move |slot: &Path| {
                if !changed && slot.file_name() == Some(OsStr::new("new")) {
                    changed = true;
                    fs::rename(&changed_source, &retained).unwrap();
                    fs::write(&changed_source, b"foreign new source").unwrap();
                }
            };
            let mut names = PortableNames::default();
            if after_guard {
                names.capturing = Some(Box::new(move |_, slot| race(slot)));
            } else {
                names.reserved = Some(Box::new(race));
            }
            let mut progress = super::super::tests::progress();
            let result = replace_move_using(
                &source,
                &target,
                &consent,
                &expected,
                &mut progress,
                &mut names,
            );
            if after_guard {
                assert_eq!(result.unwrap(), Some(false));
            } else {
                assert_eq!(result.unwrap_err().raw_os_error(), Some(libc::EBUSY));
            }
            assert_eq!(fs::read(source).unwrap(), b"foreign new source");
            assert_eq!(
                fs::read(src.join("consented-original")).unwrap(),
                b"consented source"
            );
            assert_eq!(fs::read(target).unwrap(), b"old target");
            assert_eq!(progress.done, 0);
            assert!(progress.recovery_paths.is_empty());
        }
    }

    #[test]
    fn portable_publication_preserves_a_foreign_target_and_reports_the_captured_old_target() {
        let f = Fixture::new();
        let src = f.folder("src");
        let dest = f.folder("dest");
        let source = src.join("file");
        let target = dest.join("file");
        fs::write(&source, b"restored source").unwrap();
        fs::write(&target, b"recover old target").unwrap();
        let consent = fs::symlink_metadata(&source).unwrap();
        let expected = fs::symlink_metadata(&target).unwrap();
        let new_target = target.clone();
        let mut names = PortableNames {
            captured: Some(Box::new(move |_, slot| {
                if slot.file_name() == Some(OsStr::new("old")) {
                    fs::write(&new_target, b"foreign target must remain").unwrap();
                }
            })),
            ..Default::default()
        };
        let mut progress = super::super::tests::progress();
        assert_eq!(
            replace_move_using(
                &source,
                &target,
                &consent,
                &expected,
                &mut progress,
                &mut names
            )
            .unwrap_err()
            .raw_os_error(),
            Some(libc::EBUSY)
        );
        assert_eq!(fs::read(source).unwrap(), b"restored source");
        assert_eq!(fs::read(target).unwrap(), b"foreign target must remain");
        let old = progress
            .recovery_paths
            .iter()
            .find(|path| path.file_name() == Some(OsStr::new("old")))
            .unwrap();
        assert_eq!(fs::read(old).unwrap(), b"recover old target");
        assert_eq!(progress.done, 0);
    }

    #[test]
    fn portable_rollback_never_replaces_an_occupied_source_and_keeps_its_private_copy() {
        let f = Fixture::new();
        let src = f.folder("src");
        let dest = f.folder("dest");
        let source = src.join("file");
        let target = dest.join("file");
        fs::write(&source, b"source in recovery").unwrap();
        fs::write(&target, b"target restored").unwrap();
        let consent = fs::symlink_metadata(&source).unwrap();
        let expected = fs::symlink_metadata(&target).unwrap();
        let occupied = source.clone();
        let mut names = PortableNames {
            captured: Some(Box::new(move |_, slot| {
                if slot.file_name() == Some(OsStr::new("new")) {
                    fs::write(&occupied, b"foreign source must remain").unwrap();
                }
            })),
            link_error: Some(libc::ENOSPC),
            ..Default::default()
        };
        let mut progress = super::super::tests::progress();
        assert_eq!(
            replace_move_using(
                &source,
                &target,
                &consent,
                &expected,
                &mut progress,
                &mut names
            )
            .unwrap_err()
            .raw_os_error(),
            Some(libc::EBUSY)
        );
        assert_eq!(fs::read(source).unwrap(), b"foreign source must remain");
        assert_eq!(fs::read(target).unwrap(), b"target restored");
        let new = progress
            .recovery_paths
            .iter()
            .find(|path| path.file_name() == Some(OsStr::new("new")))
            .unwrap();
        assert_eq!(fs::read(new).unwrap(), b"source in recovery");
        assert_eq!(progress.done, 0);
    }

    #[test]
    fn portable_link_cleanup_failure_preserves_the_alias_and_counts_published_bytes_once() {
        let f = Fixture::new();
        let src = f.folder("src");
        let dest = f.folder("dest");
        let source = src.join("file");
        let target = dest.join("file");
        fs::write(&source, b"published source").unwrap();
        fs::write(&target, b"old target").unwrap();
        let consent = fs::symlink_metadata(&source).unwrap();
        let expected = fs::symlink_metadata(&target).unwrap();
        let mut names = PortableNames {
            remove_error: Some("new"),
            ..Default::default()
        };
        let mut progress = super::super::tests::progress();
        assert_eq!(
            replace_move_using(
                &source,
                &target,
                &consent,
                &expected,
                &mut progress,
                &mut names
            )
            .unwrap(),
            Some(true)
        );
        assert!(!lexists(&source));
        assert_eq!(fs::read(&target).unwrap(), b"published source");
        let alias = progress
            .recovery_paths
            .iter()
            .find(|path| path.file_name() == Some(OsStr::new("new")))
            .unwrap();
        assert!(same_identity(
            &fs::symlink_metadata(alias).unwrap(),
            &fs::symlink_metadata(&target).unwrap()
        ));
        assert_eq!(fs::read(alias).unwrap(), b"published source");
        assert_eq!(progress.done, consent.len());
        assert_eq!(progress.entries_done, Some(1));
        assert_eq!(progress.overwritten_paths, vec![target]);
    }

    #[test]
    fn portable_publication_enospc_or_exdev_restores_both_entries_without_retrying_a_partial_move()
    {
        for errno in [libc::ENOSPC, libc::EXDEV] {
            let f = Fixture::new();
            let src = f.folder("src");
            let dest = f.folder("dest");
            let source = src.join("file");
            let target = dest.join("file");
            fs::write(&source, b"source restored").unwrap();
            fs::write(&target, b"target restored").unwrap();
            let consent = fs::symlink_metadata(&source).unwrap();
            let expected = fs::symlink_metadata(&target).unwrap();
            let mut names = PortableNames {
                link_error: Some(errno),
                ..Default::default()
            };
            let mut progress = super::super::tests::progress();
            assert_eq!(
                replace_move_using(
                    &source,
                    &target,
                    &consent,
                    &expected,
                    &mut progress,
                    &mut names
                )
                .unwrap_err()
                .raw_os_error(),
                Some(errno)
            );
            assert!(captured_matches(
                &consent,
                &fs::symlink_metadata(&source).unwrap()
            ));
            assert!(captured_matches(
                &expected,
                &fs::symlink_metadata(&target).unwrap()
            ));
            assert_eq!(fs::read(source).unwrap(), b"source restored");
            assert_eq!(fs::read(target).unwrap(), b"target restored");
            assert_eq!(progress.done, 0);
            assert!(progress.overwritten_paths.is_empty());
            assert!(progress.recovery_paths.is_empty());
            assert_eq!(fs::read_dir(dest).unwrap().count(), 1);
        }
    }

    #[test]
    fn portable_directory_over_file_is_conservatively_rejected_and_the_old_file_restored() {
        let f = Fixture::new();
        let source = f.folder("src/tree");
        let dest = f.folder("dest");
        let target = dest.join("tree");
        fs::write(source.join("child"), b"directory source stays").unwrap();
        fs::write(&target, b"old file stays").unwrap();
        let consent = fs::symlink_metadata(&source).unwrap();
        let expected = fs::symlink_metadata(&target).unwrap();
        let mut progress = super::super::tests::progress();
        let mut names = PortableNames::default();
        assert_eq!(
            replace_copy_using(
                &source,
                &target,
                &consent,
                &expected,
                true,
                &mut progress,
                &mut names
            )
            .unwrap_err()
            .raw_os_error(),
            Some(libc::EXDEV)
        );
        assert_eq!(
            fs::read(source.join("child")).unwrap(),
            b"directory source stays"
        );
        assert_eq!(fs::read(target).unwrap(), b"old file stays");
        assert_eq!(progress.done, 0);
        assert!(progress.recovery_paths.is_empty());
        assert_eq!(fs::read_dir(dest).unwrap().count(), 1);
    }
}
