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
        Err(error) if error.raw_os_error() == Some(libc::EXDEV) => {
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
    let parent = target
        .parent()
        .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?;
    let stage_root = parent.join(format!(".winpoint-transfer-{}", nonce()));
    fs::DirBuilder::new().mode(0o700).create(&stage_root)?;
    let stage = opened_directory(&stage_root)?;
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
        match rename_noreplace(target, &old) {
            Ok(()) => captured = true,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        }
        let observed = fs::symlink_metadata(&old)?;
        if !captured_matches(expected, &observed) {
            if rename_noreplace(&old, target).is_ok() {
                captured = false;
                return Ok(false);
            }
            return Err(io::Error::from_raw_os_error(libc::EBUSY));
        }
        old_metadata = Some(observed);
        rename_noreplace(&new, target)?; // a newly raced target is never overwritten
        published = true;
        let published_metadata = fs::symlink_metadata(target)?;
        if !captured_matches(&snapshot.copied, &published_metadata) {
            return Err(io::Error::from_raw_os_error(libc::EBUSY));
        }
        snapshot.copied = published_metadata;
        if moving {
            remove_source(source, target, &snapshot)?;
        }
        record(&mut progress.overwritten_paths, target);
        drop(commit);
        Ok(true)
    })();
    if !published {
        let changed = !unchanged(consent_source, &snapshot.metadata)
            || !fs::symlink_metadata(source).is_ok_and(|now| unchanged(consent_source, &now));
        progress.discard_copy(mark, &new, cleanup_snapshot(&new, &snapshot), changed);
        if captured && rename_noreplace(&old, target).is_ok() {
            captured = false;
        }
    }
    if captured {
        if matches!(result, Ok(true))
            && let Some(metadata) = &old_metadata
        {
            if cleanup_owned(&old, metadata).is_ok() {
                captured = false;
            }
        }
        if captured {
            record(&mut progress.recovery_paths, &old);
        }
    }
    if fs::remove_dir(&stage_root).is_err() && lexists(&stage_root) {
        record(&mut progress.recovery_paths, &stage_root);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

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
        let contents = vec![0x44; 4 * 1024 * 1024];
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
        // Inject ENOSPC once only after one real MiB has been written, at the I/O error boundary.
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
}
