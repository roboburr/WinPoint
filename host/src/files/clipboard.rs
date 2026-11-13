//! The file clipboard belongs to the user session, rather than one Explorer process.
use super::{ApiError, ApiResult, Context, directory, io_error, mutate, paths};
use crate::events::{nonce, private_dir};
use crate::i18n::Lang;
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};

const STATE_LIMIT: usize = 32 * 1024 * 1024;
#[derive(Clone)]
pub(super) struct Clipboard {
    root: PathBuf,
}
impl Clipboard {
    pub fn new(root: PathBuf) -> Result<Self, ApiError> {
        // A start-up error (Files::new prints it on stderr): the process's language.
        let lang = crate::i18n::current_language();
        private_dir(&root).map_err(|error| io_error(error, &root, lang))?;
        Ok(Self { root })
    }
    fn lock(&self, lang: Lang) -> Result<Lock, ApiError> {
        let path = self.root.join("clipboard.lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
            .map_err(|error| io_error(error, &path, lang))?;
        check_private(&file, &path, lang)?;
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
                return Ok(Lock(file));
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted {
                return Err(io_error(error, &path, lang));
            }
        }
    }
    fn read(&self, lang: Lang) -> ApiResult {
        let path = self.root.join("clipboard.json");
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(json!({"paths":[],"op":"copy","revision":""}));
            }
            Err(error) => return Err(io_error(error, &path, lang)),
        };
        check_private(&file, &path, lang)?;
        let mut bytes = Vec::new();
        file.take((STATE_LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| io_error(error, &path, lang))?;
        if bytes.len() > STATE_LIMIT {
            return Err(ApiError::tr(
                "failed",
                lang,
                "files.clipboardTooLarge",
                crate::i18n::NO_PARAMS,
            ));
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
            ApiError::tr(
                "failed",
                lang,
                "files.clipboardUnreadable",
                crate::i18n::NO_PARAMS,
            )
        })?;
        if !matches!(value["op"].as_str(), Some("copy" | "cut"))
            || !value["paths"].as_array().is_some_and(|paths| {
                paths.len() <= 10_000
                    && paths.iter().all(|path| {
                        path.as_str()
                            .is_some_and(|path| path.starts_with('/') && !path.contains('\0'))
                    })
            })
        {
            return Err(ApiError::tr(
                "failed",
                lang,
                "files.clipboardInvalid",
                crate::i18n::NO_PARAMS,
            ));
        }
        Ok(value)
    }
    fn write(&self, value: &Value, lang: Lang) -> Result<u64, ApiError> {
        let sequence = self.read(lang)?["sequence"]
            .as_u64()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| {
                ApiError::tr(
                    "failed",
                    lang,
                    "files.clipboardNotUpdated",
                    crate::i18n::NO_PARAMS,
                )
            })?;
        let mut value = value.clone();
        value["sequence"] = json!(sequence);
        let bytes = value.to_string().into_bytes();
        if bytes.len() > STATE_LIMIT {
            return Err(ApiError::tr(
                "bad_args",
                lang,
                "files.selectionTooLarge",
                crate::i18n::NO_PARAMS,
            ));
        }
        let temporary = self.root.join(format!(".clipboard-{}.tmp", nonce()));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&temporary)
                .map_err(|error| io_error(error, &temporary, lang))?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|error| io_error(error, &temporary, lang))?;
            fs::rename(&temporary, self.root.join("clipboard.json"))
                .map_err(|error| io_error(error, &temporary, lang))?;
            Ok(sequence)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }
    pub fn event_snapshot(&self) -> Option<Value> {
        // Errors are dropped here, so their language does not matter.
        let lang = Lang::Nl;
        let _lock = self.lock(lang).ok()?;
        let state = self.read(lang).ok()?;
        Some(
            json!({"paths":state["paths"],"op":state["op"],"_revision":state["sequence"].as_u64().unwrap_or(0)}),
        )
    }
    pub fn call(&self, method: &str, args: &Value, ctx: &Context) -> Option<ApiResult> {
        let lang = ctx.lang();
        match method {
            "files.clipboardSet" => Some((|| {
                let op = match &args["op"] {
                    Value::Null | Value::Bool(false) => "copy",
                    Value::String(value) if value.is_empty() || value == "copy" => "copy",
                    Value::Number(value) if value.as_f64() == Some(0.0) => "copy",
                    Value::Array(value) if value.is_empty() => "copy",
                    Value::Object(value) if value.is_empty() => "copy",
                    Value::String(value) if value == "cut" => "cut",
                    _ => {
                        return Err(ApiError::tr(
                            "bad_args",
                            lang,
                            "files.badOp",
                            crate::i18n::NO_PARAMS,
                        ));
                    }
                };
                let paths: Vec<_> = paths(&args["paths"], lang)?
                    .into_iter()
                    .filter(|path| fs::symlink_metadata(path).is_ok())
                    .collect();
                let lock = self.lock(lang)?;
                self.read(lang)?;
                let sequence =
                    self.write(&json!({"paths":paths,"op":op,"revision":nonce()}), lang)?;
                drop(lock);
                let result = json!({"paths":paths,"op":op,"_revision":sequence});
                ctx.emit(
                    "files.clipboardChanged",
                    json!({"paths":paths,"op":op,"_revision":sequence}),
                );
                Ok(result)
            })()),
            "files.clipboardGet" => Some((|| {
                let _lock = self.lock(lang)?;
                let state = self.read(lang)?;
                ctx.observe_clipboard(state["sequence"].as_u64().unwrap_or(0));
                let mut result = existing(&state);
                result["_revision"] = json!(state["sequence"].as_u64().unwrap_or(0));
                Ok(result)
            })()),
            "files.clipboardClear" => Some((|| {
                let lock = self.lock(lang)?;
                self.read(lang)?;
                let sequence =
                    self.write(&json!({"paths":[],"op":"copy","revision":nonce()}), lang)?;
                drop(lock);
                ctx.emit(
                    "files.clipboardChanged",
                    json!({"paths":[],"op":"copy","_revision":sequence}),
                );
                Ok(json!({}))
            })()),
            "files.paste" => Some((|| {
                let destination = directory(&args["dest"], lang)?;
                let lock = self.lock(lang)?;
                let state = self.read(lang)?;
                let selected = existing(&state);
                drop(lock);
                let selected: Vec<PathBuf> = selected["paths"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(Value::as_str)
                    .map(PathBuf::from)
                    .collect();
                if selected.is_empty() {
                    return Err(ApiError::tr(
                        "empty",
                        lang,
                        "files.nothingToPaste",
                        crate::i18n::NO_PARAMS,
                    ));
                }
                let cutting = state["op"] == "cut";
                let result =
                    mutate::transfer(selected, destination, cutting, args["jobId"].as_str(), ctx)?;
                if cutting {
                    let lock = self.lock(lang)?;
                    // A user may copy another selection while a large move is running.
                    // Clear only the selection this paste actually consumed.
                    if self.read(lang)?["revision"] == state["revision"] {
                        let sequence =
                            self.write(&json!({"paths":[],"op":"copy","revision":nonce()}), lang)?;
                        drop(lock);
                        ctx.emit(
                            "files.clipboardChanged",
                            json!({"paths":[],"op":"copy","_revision":sequence}),
                        );
                    }
                }
                Ok(result)
            })()),
            _ => None,
        }
    }
}

fn existing(state: &Value) -> Value {
    let paths: Vec<_> = state["paths"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|value| {
            value
                .as_str()
                .is_some_and(|path| fs::symlink_metadata(path).is_ok())
        })
        .cloned()
        .collect();
    json!({"paths":paths,"op":state["op"]})
}
fn check_private(file: &File, path: &Path, lang: Lang) -> Result<(), ApiError> {
    let metadata = file
        .metadata()
        .map_err(|error| io_error(error, path, lang))?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
    {
        return Err(ApiError::tr(
            "no_access",
            lang,
            "files.clipboardNotPrivate",
            crate::i18n::NO_PARAMS,
        ));
    }
    Ok(())
}
struct Lock(File);
impl Drop for Lock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    #[test]
    fn independent_hosts_share_copy_cut_and_broken_symlink_state() {
        let root = std::env::temp_dir().join(format!("win11-clipboard-{}", nonce()));
        fs::create_dir(&root).unwrap();
        let source = root.join("source");
        let dest = root.join("dest");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&dest).unwrap();
        fs::write(source.join("a.txt"), b"hello").unwrap();
        symlink("missing", source.join("broken")).unwrap();
        let first = Clipboard::new(root.join("state")).unwrap();
        let second = Clipboard::new(root.join("state")).unwrap();
        let ctx = Context::new(root.clone(), root.join("Desktop"), "en".into(), false);
        let args = json!({"paths":[source.join("a.txt"),source.join("broken")],"op":"cut"});
        first
            .call("files.clipboardSet", &args, &ctx)
            .unwrap()
            .unwrap();
        assert_eq!(
            second
                .call("files.clipboardGet", &json!({}), &ctx)
                .unwrap()
                .unwrap()["paths"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        second
            .call("files.paste", &json!({"dest":dest}), &ctx)
            .unwrap()
            .unwrap();
        assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"hello");
        assert_eq!(
            fs::read_link(dest.join("broken")).unwrap(),
            PathBuf::from("missing")
        );
        assert!(
            first
                .call("files.clipboardGet", &json!({}), &ctx)
                .unwrap()
                .unwrap()["paths"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(!source.join("a.txt").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
