//! ZIP creation and ZIP/TAR extraction with the reference Explorer API and transactional publish.
use super::{ApiError, ApiResult, Context, abs_in, directory, io_error, mutate, paths};
use crate::i18n::Lang;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    ffi::{CStr, CString, OsStr, OsString},
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::{OsStrExt, OsStringExt},
            fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
        },
    },
    path::{Component, Path, PathBuf},
    ptr,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
#[path = "archive_ffi.rs"]
mod ffi;

const OK: i32 = 0;
const EOF: i32 = 1;
const WARN: i32 = -20;
const TAR: i32 = 0x30000;
const CHUNK: usize = 1024 * 1024;

pub(super) fn call(method: &str, args: &Value, ctx: &Context) -> Option<ApiResult> {
    match method {
        "files.compress" => Some(compress(args, ctx)),
        "files.extract" => Some(extract(args, ctx)),
        _ => None,
    }
}

/// files.badArchive with `detail` (a diagnostic text) as its data.
fn bad_archive(detail: impl Into<String>, lang: Lang) -> ApiError {
    let mut error = ApiError::tr(
        "bad_archive",
        lang,
        "files.badArchive",
        crate::i18n::NO_PARAMS,
    );
    error.data = Some(json!(detail.into()));
    error
}
fn unsupported(lang: Lang) -> ApiError {
    ApiError::tr(
        "unsupported_archive",
        lang,
        "files.unsupportedArchive",
        crate::i18n::NO_PARAMS,
    )
}
fn unsafe_path(lang: Lang) -> ApiError {
    ApiError::tr(
        "unsafe_archive",
        lang,
        "files.unsafePaths",
        crate::i18n::NO_PARAMS,
    )
}
fn unsafe_link(lang: Lang) -> ApiError {
    ApiError::tr(
        "unsafe_archive",
        lang,
        "files.unsafeLinks",
        crate::i18n::NO_PARAMS,
    )
}
fn unsafe_device(lang: Lang) -> ApiError {
    ApiError::tr(
        "unsafe_archive",
        lang,
        "files.deviceFiles",
        crate::i18n::NO_PARAMS,
    )
}
fn library(lang: Lang) -> Result<&'static ffi::Library, ApiError> {
    ffi::library().map_err(|error| load_error(error, lang))
}
/// The envelope of a libarchive load error, with `key` + `params` like every catalogue message (C2). For a
/// missing symbol the key names the first piece of "<piece><symbol>" (3b898e6 built it so), and `message` is the
/// whole sentence.
fn load_error(error: &ffi::LoadError, lang: Lang) -> ApiError {
    let mut api = match error {
        ffi::LoadError::Missing => ApiError::tr(
            "unsupported_archive",
            lang,
            "files.libarchiveMissing",
            crate::i18n::NO_PARAMS,
        ),
        ffi::LoadError::Symbol(_) => ApiError::tr(
            "unsupported_archive",
            lang,
            "files.libarchiveLacks",
            crate::i18n::NO_PARAMS,
        ),
    };
    api.message = error.message(lang);
    api
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
fn nonce() -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
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
fn c_path(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))
}

struct Progress<'a> {
    ctx: &'a Context,
    job: Value,
    op: &'static str,
    total: Option<u64>,
    done: u64,
    items: usize,
    count: usize,
    current: String,
    last: Option<Instant>,
    finished: bool,
}
impl<'a> Progress<'a> {
    fn new(
        ctx: &'a Context,
        op: &'static str,
        job: &Value,
        total: Option<u64>,
        count: usize,
    ) -> Self {
        Self {
            ctx,
            job: if truthy(job) {
                job.clone()
            } else {
                json!(nonce())
            },
            op,
            total,
            done: 0,
            items: 0,
            count,
            current: String::new(),
            last: None,
            finished: false,
        }
    }
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
        self.ctx.emit("files.progress",json!({"jobId":self.job,"op":self.op,"done":self.done,"total":self.total,"items":self.items,"count":self.count,"current":self.current,"finished":finished}));
    }
    fn add(&mut self, bytes: u64) {
        self.done = self.done.saturating_add(bytes);
        self.emit(false, false);
    }
    fn finish(&mut self) {
        if !self.finished {
            self.emit(true, true);
            self.finished = true;
        }
    }
}
impl Drop for Progress<'_> {
    fn drop(&mut self) {
        self.finish();
    }
}

struct Staging {
    container: PathBuf,
    payload: PathBuf,
    owned: Metadata,
}
impl Staging {
    fn new(directory: &Path, name: &str, is_dir: bool) -> io::Result<Self> {
        for _ in 0..20 {
            let container = directory.join(format!(".{name}.{}.partial", nonce()));
            match fs::DirBuilder::new().mode(0o700).create(&container) {
                Ok(()) => {
                    let owned = fs::symlink_metadata(&container)?;
                    let payload = container.join("payload");
                    let stage = Self {
                        container,
                        payload,
                        owned,
                    };
                    if is_dir {
                        fs::DirBuilder::new().mode(0o700).create(&stage.payload)?;
                    }
                    return Ok(stage);
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::from_raw_os_error(libc::EEXIST))
    }
    fn publish(&self, initial: &Path, is_dir: bool) -> io::Result<PathBuf> {
        let directory = initial.parent().unwrap_or(Path::new("/"));
        let mut target = initial.to_path_buf();
        for _ in 0..50 {
            match mutate::rename_noreplace(&self.payload, &target) {
                Ok(()) => return Ok(target),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    target = mutate::unique_path(
                        directory,
                        target
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .as_ref(),
                        is_dir,
                    )?;
                }
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::from_raw_os_error(libc::EEXIST))
    }
}
impl Drop for Staging {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.container).is_ok_and(|metadata| {
            metadata.dev() == self.owned.dev() && metadata.ino() == self.owned.ino()
        }) {
            let _ = fs::remove_dir_all(&self.container);
        }
    }
}

fn compress(args: &Value, ctx: &Context) -> ApiResult {
    let lang = ctx.lang();
    let selected = paths(&args["paths"], lang)?;
    for path in &selected {
        if fs::symlink_metadata(path).is_err() {
            return Err(ApiError::tr(
                "not_found",
                lang,
                "files.namedGone",
                &serde_json::json!({"name": path.file_name().unwrap_or_default().to_string_lossy().to_string()}),
            ));
        }
    }
    let first = &selected[0];
    let out = if truthy(&args["dest"]) {
        directory(&args["dest"], lang)?
    } else {
        first.parent().unwrap_or(Path::new("/")).into()
    };
    let name = first.file_name().unwrap_or_default().to_string_lossy();
    let stem = if first.is_dir() {
        name.as_ref()
    } else {
        mutate::split_name(&name).0
    };
    let name = format!("{stem}.zip");
    let target =
        mutate::unique_path(&out, &name, false).map_err(|error| io_error(error, &out, lang))?;
    let mut progress = Progress::new(
        ctx,
        "compress",
        &args["jobId"],
        total_size(&selected),
        selected.len(),
    );
    let stage = Staging::new(
        &out,
        target
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .as_ref(),
        false,
    )
    .map_err(|error| io_error(error, &target, lang))?;
    let lib = library(lang)?;
    let mut writer = ArchiveWriter::zip(lib, &stage.payload, lang)
        .map_err(|error| io_error(error, &target, lang))?;
    for path in &selected {
        zip_tree(
            &mut writer,
            path,
            Path::new(path.file_name().unwrap_or_default()),
            &stage.container,
            &mut progress,
        )
        .map_err(|error| io_error(error, &target, lang))?;
        progress.items += 1;
    }
    writer
        .finish()
        .map_err(|error| io_error(error, &target, lang))?;
    let published = stage
        .publish(&target, false)
        .map_err(|error| io_error(error, &target, lang))?;
    progress.finish();
    ctx.changed(&out);
    Ok(json!({"path":published}))
}

fn total_size(paths: &[PathBuf]) -> Option<u64> {
    let deadline = Instant::now() + Duration::from_millis(1500);
    let mut total = 0_u64;
    for path in paths {
        let (bytes, _, _, complete) = super::properties::du(path, deadline);
        total = total.saturating_add(bytes);
        if !complete {
            return None;
        }
    }
    Some(total)
}

struct ArchiveWriter {
    lib: &'static ffi::Library,
    handle: ffi::Handle,
    _file: File,
    closed: bool,
    lang: Lang,
}
impl ArchiveWriter {
    fn zip(lib: &'static ffi::Library, path: &Path, lang: Lang) -> io::Result<Self> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o666)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        let handle = unsafe { (lib.archive_write_new)() };
        if handle.is_null() {
            return Err(io::Error::from_raw_os_error(libc::ENOMEM));
        }
        let writer = Self {
            lib,
            handle,
            _file: file,
            closed: false,
            lang,
        };
        for status in [
            unsafe { (lib.archive_write_set_format_zip)(handle) },
            unsafe {
                (lib.archive_write_set_options)(
                    handle,
                    c"zip:compression=deflate,zip:compression-level=6".as_ptr(),
                )
            },
            unsafe { (lib.archive_write_set_bytes_per_block)(handle, 0) },
            unsafe { (lib.archive_write_set_bytes_in_last_block)(handle, 1) },
            unsafe { (lib.archive_write_open_fd)(handle, writer._file.as_raw_fd()) },
        ] {
            writer.check(status)?;
        }
        Ok(writer)
    }
    fn check(&self, status: i32) -> io::Result<()> {
        if status >= OK {
            Ok(())
        } else {
            let errno = unsafe { (self.lib.archive_errno)(self.handle) };
            if errno > 0 {
                Err(io::Error::from_raw_os_error(errno))
            } else {
                Err(io::Error::other(self.lib.error(self.handle, self.lang)))
            }
        }
    }
    fn entry(&self, name: &Path, metadata: &Metadata) -> io::Result<()> {
        let text = name
            .to_str()
            .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?;
        let name = CString::new(if metadata.is_dir() {
            format!("{}/", text.trim_end_matches('/'))
        } else {
            text.into()
        })
        .map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?;
        let entry = unsafe { (self.lib.archive_entry_new)() };
        if entry.is_null() {
            return Err(io::Error::from_raw_os_error(libc::ENOMEM));
        }
        unsafe {
            (self.lib.archive_entry_set_pathname_utf8)(entry, name.as_ptr());
            (self.lib.archive_entry_set_mode)(entry, metadata.mode());
            (self.lib.archive_entry_set_size)(
                entry,
                if metadata.is_dir() {
                    0
                } else {
                    metadata.len() as i64
                },
            );
            (self.lib.archive_entry_set_mtime)(entry, metadata.mtime(), metadata.mtime_nsec());
        }
        let result = self.check(unsafe { (self.lib.archive_write_header)(self.handle, entry) });
        unsafe { (self.lib.archive_entry_free)(entry) };
        result
    }
    fn data(&self, mut bytes: &[u8]) -> io::Result<()> {
        while !bytes.is_empty() {
            let written = unsafe {
                (self.lib.archive_write_data)(self.handle, bytes.as_ptr().cast(), bytes.len())
            };
            if written <= 0 {
                self.check(written as i32)?;
                return Err(io::Error::from_raw_os_error(libc::EIO));
            }
            bytes = &bytes[written as usize..];
        }
        Ok(())
    }
    fn finish(&mut self) -> io::Result<()> {
        if self.closed {
            return Ok(());
        }
        let status = unsafe { (self.lib.archive_write_close)(self.handle) };
        self.closed = true;
        self.check(status)
    }
}
impl Drop for ArchiveWriter {
    fn drop(&mut self) {
        unsafe { (self.lib.archive_write_free)(self.handle) };
    }
}

fn zip_tree(
    writer: &mut ArchiveWriter,
    path: &Path,
    name: &Path,
    exclude: &Path,
    progress: &mut Progress<'_>,
) -> io::Result<()> {
    if path == exclude {
        return Ok(());
    }
    let link = fs::symlink_metadata(path)?;
    let metadata = if link.file_type().is_symlink() {
        match fs::metadata(path) {
            Ok(metadata) if !metadata.is_dir() => metadata,
            _ => return Ok(()),
        }
    } else {
        link
    };
    if metadata.is_dir() {
        writer.entry(name, &metadata)?;
        let mut entries = fs::read_dir(path)?.collect::<io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            zip_tree(
                writer,
                &entry.path(),
                &name.join(entry.file_name()),
                exclude,
                progress,
            )?;
        }
    } else if metadata.is_file() {
        let mut input = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)?;
        let opened = input.metadata()?;
        if !opened.is_file() || opened.dev() != metadata.dev() || opened.ino() != metadata.ino() {
            return Err(io::Error::from_raw_os_error(libc::EBUSY));
        }
        progress.current = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        writer.entry(name, &opened)?;
        let mut buffer = vec![0_u8; (opened.len().min(CHUNK as u64) as usize).max(1)];
        let mut copied = 0_u64;
        loop {
            let n = input.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            writer.data(&buffer[..n])?;
            copied += n as u64;
        }
        if copied != opened.len() {
            return Err(io::Error::from_raw_os_error(libc::EBUSY));
        }
        progress.add(opened.len());
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Zip,
    Tar,
}
#[derive(Clone)]
struct ZipEntry {
    name: String,
    size: u64,
}
#[derive(Clone, Default)]
struct ZipIndex {
    entries: BTreeMap<u64, ZipEntry>,
    patches: Vec<(u64, u8)>,
}
fn u16le(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
}
fn u32le(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}
fn u64le(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}
fn exact_at(file: &mut File, offset: u64, length: usize) -> io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0_u8; length];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn zip_index(file: &mut File, lang: Lang) -> Result<Option<ZipIndex>, ApiError> {
    let length = file
        .metadata()
        .map_err(|error| bad_archive(error.to_string(), lang))?
        .len();
    let tail_len = length.min(65_557) as usize;
    if tail_len < 22 {
        return Ok(None);
    }
    let tail = exact_at(file, length - tail_len as u64, tail_len)
        .map_err(|error| bad_archive(error.to_string(), lang))?;
    let Some(end) = (0..=tail.len() - 22)
        .rev()
        .find(|at| tail[*at..].starts_with(b"PK\x05\x06"))
    else {
        return Ok(None);
    };
    let end_absolute = length - tail_len as u64 + end as u64;
    let mut count = u16le(&tail, end + 10) as u64;
    let mut central_size = u32le(&tail, end + 12) as u64;
    let mut central_offset = u32le(&tail, end + 16) as u64;
    let mut central_end = end_absolute;
    if u16le(&tail, end + 4) != 0 || u16le(&tail, end + 6) != 0 {
        return Err(bad_archive(
            &crate::i18n::tr(lang, "files.zipMultiDisk"),
            lang,
        ));
    }
    if end_absolute >= 20 {
        let locator = exact_at(file, end_absolute - 20, 20)
            .map_err(|error| bad_archive(error.to_string(), lang))?;
        if locator.starts_with(b"PK\x06\x07") {
            let offset = u64le(&locator, 8);
            let record =
                exact_at(file, offset, 56).map_err(|error| bad_archive(error.to_string(), lang))?;
            if !record.starts_with(b"PK\x06\x06")
                || u32le(&record, 16) != 0
                || u32le(&record, 20) != 0
            {
                return Err(bad_archive(
                    &crate::i18n::tr(lang, "files.zip64IndexInvalid"),
                    lang,
                ));
            }
            count = u64le(&record, 32);
            central_size = u64le(&record, 40);
            central_offset = u64le(&record, 48);
            central_end = offset;
        }
    }
    let start = central_end
        .checked_sub(central_size)
        .ok_or_else(|| bad_archive(&crate::i18n::tr(lang, "files.zipIndexInvalid"), lang))?;
    let prefix = start
        .checked_sub(central_offset)
        .ok_or_else(|| bad_archive(&crate::i18n::tr(lang, "files.zipOffsetInvalid"), lang))?;
    let mut index = ZipIndex::default();
    let mut cursor = start;
    for _ in 0..count {
        let header =
            exact_at(file, cursor, 46).map_err(|error| bad_archive(error.to_string(), lang))?;
        if !header.starts_with(b"PK\x01\x02") {
            return Err(bad_archive(
                &crate::i18n::tr(lang, "files.zipIndexInvalid"),
                lang,
            ));
        }
        let name_length = u16le(&header, 28) as usize;
        let extra_length = u16le(&header, 30) as usize;
        let comment_length = u16le(&header, 32) as usize;
        let raw = exact_at(file, cursor + 46, name_length + extra_length)
            .map_err(|error| bad_archive(error.to_string(), lang))?;
        let filename = &raw[..name_length];
        let filename = &filename[..filename
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(filename.len())];
        let name = if u16le(&header, 8) & 0x800 != 0 {
            String::from_utf8(filename.to_vec())
                .map_err(|error| bad_archive(error.to_string(), lang))?
        } else {
            let (utf8, _) = gtk::glib::convert(filename, "UTF-8", "CP437")
                .map_err(|error| bad_archive(error.to_string(), lang))?;
            String::from_utf8(utf8.to_vec())
                .map_err(|error| bad_archive(error.to_string(), lang))?
        };
        if !safe_member(&name) {
            return Err(unsafe_path(lang));
        }
        let mut size = u32le(&header, 24) as u64;
        let mut offset = u32le(&header, 42) as u64;
        let compressed = u32le(&header, 20);
        let mut extra_at = name_length;
        while extra_at + 4 <= raw.len() {
            let field = u16le(&raw, extra_at);
            let n = u16le(&raw, extra_at + 2) as usize;
            extra_at += 4;
            if extra_at + n > raw.len() {
                return Err(bad_archive(
                    &crate::i18n::tr(lang, "files.zipExtraInvalid"),
                    lang,
                ));
            }
            if field == 1 {
                let mut at = extra_at;
                if size == u32::MAX as u64 {
                    if at + 8 > extra_at + n {
                        return Err(bad_archive(
                            &crate::i18n::tr(lang, "files.zip64SizeInvalid"),
                            lang,
                        ));
                    }
                    size = u64le(&raw, at);
                    at += 8;
                }
                if compressed == u32::MAX {
                    at += 8;
                }
                if offset == u32::MAX as u64 {
                    if at + 8 > extra_at + n {
                        return Err(bad_archive(
                            &crate::i18n::tr(lang, "files.zip64OffsetInvalid"),
                            lang,
                        ));
                    }
                    offset = u64le(&raw, at);
                }
            }
            extra_at += n;
        }
        let offset = offset
            .checked_add(prefix)
            .ok_or_else(|| bad_archive(&crate::i18n::tr(lang, "files.zipOffsetInvalid"), lang))?;
        if index
            .entries
            .insert(
                offset,
                ZipEntry {
                    name: name.clone(),
                    size,
                },
            )
            .is_some()
        {
            return Err(bad_archive(
                &crate::i18n::tr(lang, "files.zipDuplicateHeader"),
                lang,
            ));
        }
        // Python's ZIP extraction treats Unix symlink/special entries as regular data.
        // Patch only the reader's virtual central-directory bytes, never the input file.
        let attrs = u32le(&header, 38);
        let mode = (attrs >> 16) & 0o7777;
        let attrs = if name.ends_with('/') {
            (attrs & 0xffff) | 0x10 | ((libc::S_IFDIR | mode) << 16)
        } else {
            ((attrs & 0xffff) & !0x10) | ((libc::S_IFREG | mode) << 16)
        };
        for (i, byte) in attrs.to_le_bytes().into_iter().enumerate() {
            index.patches.push((cursor + 38 + i as u64, byte));
        }
        cursor = cursor
            .checked_add((46 + name_length + extra_length + comment_length) as u64)
            .ok_or_else(|| bad_archive(&crate::i18n::tr(lang, "files.zipIndexInvalid"), lang))?;
        if cursor > central_end {
            return Err(bad_archive(
                &crate::i18n::tr(lang, "files.zipIndexLength"),
                lang,
            ));
        }
    }
    Ok(Some(index))
}

struct Input {
    file: File,
    buffer: Vec<u8>,
    position: u64,
    patches: Vec<(u64, u8)>,
    error: Option<io::Error>,
}
unsafe extern "C" fn read_callback(
    _: ffi::Handle,
    data: *mut libc::c_void,
    output: *mut *const libc::c_void,
) -> isize {
    let input = unsafe { &mut *data.cast::<Input>() };
    match input.file.read(&mut input.buffer) {
        Ok(n) => {
            let start = input.position;
            let end = start + n as u64;
            let first = input
                .patches
                .partition_point(|(position, _)| *position < start);
            for (position, byte) in input.patches[first..]
                .iter()
                .take_while(|(position, _)| *position < end)
            {
                input.buffer[(*position - start) as usize] = *byte;
            }
            input.position = end;
            unsafe { *output = input.buffer.as_ptr().cast() };
            n as isize
        }
        Err(error) => {
            input.error = Some(error);
            -1
        }
    }
}
unsafe extern "C" fn seek_callback(
    _: ffi::Handle,
    data: *mut libc::c_void,
    offset: i64,
    whence: i32,
) -> i64 {
    let input = unsafe { &mut *data.cast::<Input>() };
    let seek = match whence {
        libc::SEEK_SET if offset >= 0 => SeekFrom::Start(offset as u64),
        libc::SEEK_CUR => SeekFrom::Current(offset),
        libc::SEEK_END => SeekFrom::End(offset),
        _ => {
            input.error = Some(io::Error::from_raw_os_error(libc::EINVAL));
            return -1;
        }
    };
    match input.file.seek(seek) {
        Ok(position) => {
            input.position = position;
            position as i64
        }
        Err(error) => {
            input.error = Some(error);
            -1
        }
    }
}
unsafe extern "C" fn skip_callback(
    handle: ffi::Handle,
    data: *mut libc::c_void,
    bytes: i64,
) -> i64 {
    let input = unsafe { &mut *data.cast::<Input>() };
    let before = input.position;
    let result = unsafe { seek_callback(handle, data, bytes, libc::SEEK_CUR) };
    if result < 0 {
        -1
    } else {
        result - before as i64
    }
}

struct Reader {
    lib: &'static ffi::Library,
    handle: ffi::Handle,
    input: Box<Input>,
    zip: Option<ZipIndex>,
    lang: Lang,
}
#[derive(Clone)]
struct Entry {
    name: String,
    size: u64,
    mode: u32,
    mtime: i64,
    nsec: i64,
    kind: u32,
    symlink: Option<Vec<u8>>,
    hardlink: Option<Vec<u8>>,
}
impl Reader {
    fn new(mut file: File, zip: Option<ZipIndex>, lang: Lang) -> Result<Self, ApiError> {
        file.seek(SeekFrom::Start(0))
            .map_err(|error| bad_archive(error.to_string(), lang))?;
        let lib = library(lang)?;
        let handle = unsafe { (lib.archive_read_new)() };
        if handle.is_null() {
            return Err(bad_archive(
                &crate::i18n::tr(lang, "files.archiveReaderMemory"),
                lang,
            ));
        }
        let mut reader = Self {
            lib,
            handle,
            input: Box::new(Input {
                file,
                buffer: vec![0_u8; 64 * 1024],
                position: 0,
                patches: zip
                    .as_ref()
                    .map(|zip| zip.patches.clone())
                    .unwrap_or_default(),
                error: None,
            }),
            zip,
            lang,
        };
        let statuses = [
            unsafe { (lib.archive_read_support_filter_all)(handle) },
            if reader.zip.is_some() {
                unsafe { (lib.archive_read_support_format_zip)(handle) }
            } else {
                unsafe { (lib.archive_read_support_format_tar)(handle) }
            },
            unsafe { (lib.archive_read_set_seek_callback)(handle, Some(seek_callback)) },
        ];
        if statuses.iter().any(|status| *status < OK) {
            return Err(reader.error());
        }
        let status = unsafe {
            (lib.archive_read_open2)(
                handle,
                (&mut *reader.input as *mut Input).cast(),
                None,
                Some(read_callback),
                Some(skip_callback),
                None,
            )
        };
        if status < OK {
            return if reader.zip.is_some() {
                Err(reader.error())
            } else {
                Err(unsupported(lang))
            };
        }
        Ok(reader)
    }
    fn error(&self) -> ApiError {
        bad_archive(
            self.input
                .error
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| self.lib.error(self.handle, self.lang)),
            self.lang,
        )
    }
    fn next(&mut self) -> Result<Option<Entry>, ApiError> {
        let mut entry = ptr::null_mut();
        let status = unsafe { (self.lib.archive_read_next_header)(self.handle, &mut entry) };
        if status == EOF {
            return Ok(None);
        }
        if status < WARN || entry.is_null() {
            return Err(self.error());
        }
        let raw_name = unsafe {
            let utf8 = (self.lib.archive_entry_pathname_utf8)(entry);
            if utf8.is_null() {
                (self.lib.archive_entry_pathname)(entry)
            } else {
                utf8
            }
        };
        let name = if raw_name.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(raw_name) }
                .to_string_lossy()
                .into_owned()
        };
        let position = unsafe { (self.lib.archive_read_header_position)(self.handle) };
        let zip = self
            .zip
            .as_ref()
            .and_then(|index| index.entries.get(&(position as u64)));
        let bytes = |pointer: *const libc::c_char| {
            if pointer.is_null() {
                None
            } else {
                Some(unsafe { CStr::from_ptr(pointer) }.to_bytes().to_vec())
            }
        };
        Ok(Some(Entry {
            name: zip.map(|entry| entry.name.clone()).unwrap_or(name),
            size: zip
                .map(|entry| entry.size)
                .unwrap_or_else(|| unsafe { (self.lib.archive_entry_size)(entry) }.max(0) as u64),
            mode: unsafe { (self.lib.archive_entry_perm)(entry) },
            mtime: unsafe { (self.lib.archive_entry_mtime)(entry) },
            nsec: unsafe { (self.lib.archive_entry_mtime_nsec)(entry) },
            kind: unsafe { (self.lib.archive_entry_filetype)(entry) },
            symlink: bytes(unsafe { (self.lib.archive_entry_symlink)(entry) }),
            hardlink: bytes(unsafe { (self.lib.archive_entry_hardlink)(entry) }),
        }))
    }
    fn kind(&self) -> Result<Kind, ApiError> {
        if self.zip.is_some() {
            return Ok(Kind::Zip);
        }
        if unsafe { (self.lib.archive_format)(self.handle) } & 0xff0000 != TAR {
            return Err(unsupported(self.lang));
        }
        for n in 0..unsafe { (self.lib.archive_filter_count)(self.handle) } {
            if !matches!(
                unsafe { (self.lib.archive_filter_code)(self.handle, n) },
                0 | 1 | 2 | 5 | 6 | 14
            ) {
                return Err(unsupported(self.lang));
            }
        }
        Ok(Kind::Tar)
    }
    fn skip(&mut self) -> Result<(), ApiError> {
        if unsafe { (self.lib.archive_read_data_skip)(self.handle) } < OK {
            Err(self.error())
        } else {
            Ok(())
        }
    }
    fn data(&mut self, bytes: &mut [u8]) -> Result<usize, ApiError> {
        let n = unsafe {
            (self.lib.archive_read_data)(self.handle, bytes.as_mut_ptr().cast(), bytes.len())
        };
        if n < 0 {
            Err(self.error())
        } else {
            Ok(n as usize)
        }
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        unsafe { (self.lib.archive_read_free)(self.handle) };
    }
}

fn safe_member(name: &str) -> bool {
    let name = name.replace('\\', "/");
    !name.starts_with('/')
        && !(name.as_bytes().get(1) == Some(&b':')
            && name.as_bytes().first().is_some_and(u8::is_ascii_alphabetic))
        && !name.split('/').any(|part| part == "..")
}
fn normalized(path: &Path) -> Option<PathBuf> {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => result.push(name),
            Component::CurDir => {}
            Component::ParentDir => {
                if !result.pop() {
                    return None;
                }
            }
            _ => return None,
        }
    }
    Some(result)
}
fn normalized_link(path: &Path) -> PathBuf {
    let mut parts: Vec<OsString> = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => parts.push(name.to_owned()),
            Component::ParentDir => {
                if parts.last().is_some_and(|part| part != "..") {
                    parts.pop();
                } else {
                    parts.push(OsString::from(".."));
                }
            }
            Component::CurDir => {}
            _ => return path.to_owned(),
        }
    }
    if parts.is_empty() {
        PathBuf::from(".")
    } else {
        parts.into_iter().collect()
    }
}
fn validate(entry: &Entry, kind: Kind, lang: Lang) -> Result<(), ApiError> {
    if !safe_member(&entry.name) {
        return Err(unsafe_path(lang));
    }
    if kind == Kind::Tar {
        if let Some(link) = entry.symlink.as_ref().or(entry.hardlink.as_ref()) {
            let target = PathBuf::from(OsString::from_vec(link.clone()));
            let joined = if entry.symlink.is_some() {
                Path::new(&entry.name)
                    .parent()
                    .unwrap_or(Path::new(""))
                    .join(&target)
            } else {
                target.clone()
            };
            let joined = normalized(&joined).ok_or_else(|| unsafe_link(lang))?;
            if target.is_absolute() || !safe_member(&joined.to_string_lossy()) {
                return Err(unsafe_link(lang));
            }
        }
        if matches!(entry.kind, libc::S_IFCHR | libc::S_IFBLK | libc::S_IFIFO) {
            return Err(unsafe_device(lang));
        }
    }
    Ok(())
}

fn archive_stem(name: &str) -> String {
    const EXTS: [&str; 10] = [
        ".tar.bz2", ".tar.gz", ".tar.xz", ".tar.zst", ".zip", ".tar", ".tgz", ".tbz2", ".txz",
        ".tzst",
    ];
    let lower = name.to_lowercase();
    for ext in EXTS {
        if lower.ends_with(ext) && name.len() > ext.len() {
            return name[..name.len() - ext.len()].into();
        }
    }
    mutate::split_name(name).0.into()
}
fn same_input(first: &Metadata, second: &Metadata) -> bool {
    first.dev() == second.dev()
        && first.ino() == second.ino()
        && first.len() == second.len()
        && first.mtime() == second.mtime()
        && first.mtime_nsec() == second.mtime_nsec()
        && first.ctime() == second.ctime()
        && first.ctime_nsec() == second.ctime_nsec()
}

fn extract(args: &Value, ctx: &Context) -> ApiResult {
    let lang = ctx.lang();
    let path = abs_in(&args["path"], lang)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(&path)
        .map_err(|_| {
            ApiError::tr(
                "not_found",
                lang,
                "files.archiveGone",
                crate::i18n::NO_PARAMS,
            )
        })?;
    let metadata = file
        .metadata()
        .map_err(|error| io_error(error, &path, lang))?;
    if !metadata.is_file() {
        return Err(ApiError::tr(
            "not_found",
            lang,
            "files.archiveGone",
            crate::i18n::NO_PARAMS,
        ));
    }
    let index = zip_index(&mut file, lang)?;
    let mut reader = Reader::new(
        file.try_clone()
            .map_err(|error| io_error(error, &path, lang))?,
        index.clone(),
        lang,
    )?;
    let mut total = 0_u64;
    let mut kind = None;
    while let Some(entry) = reader.next()? {
        let current = reader.kind()?;
        kind = Some(current);
        validate(&entry, current, lang)?;
        total = total
            .checked_add(entry.size)
            .ok_or_else(|| bad_archive(&crate::i18n::tr(lang, "files.archiveTooLarge"), lang))?;
        reader.skip()?;
    }
    let kind = kind.unwrap_or(reader.kind()?);
    drop(reader);
    let out = if truthy(&args["dest"]) {
        directory(&args["dest"], lang)?
    } else {
        path.parent().unwrap_or(Path::new("/")).into()
    };
    if let Some(free) = free_space(&out)
        && total > free
    {
        return Err(ApiError::tr(
            "no_space",
            lang,
            "files.noSpaceExtract",
            crate::i18n::NO_PARAMS,
        ));
    }
    let name = archive_stem(
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .as_ref(),
    );
    let target =
        mutate::unique_path(&out, &name, true).map_err(|error| io_error(error, &out, lang))?;
    let mut progress = Progress::new(ctx, "extract", &args["jobId"], Some(total), 1);
    progress.emit(false, true);
    let stage = Staging::new(
        &out,
        target
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .as_ref(),
        true,
    )
    .map_err(|error| io_error(error, &path, lang))?;
    if !same_input(
        &metadata,
        &file
            .metadata()
            .map_err(|error| io_error(error, &path, lang))?,
    ) {
        return Err(bad_archive(
            &crate::i18n::tr(lang, "files.archiveChanged"),
            lang,
        ));
    }
    let mut reader = Reader::new(file, index, lang)?;
    extract_tree(&mut reader, &stage.payload, kind, &mut progress, &path)?;
    let published = stage
        .publish(&target, true)
        .map_err(|error| io_error(error, &path, lang))?;
    progress.finish();
    ctx.changed(&out);
    Ok(json!({"path":published}))
}

fn free_space(path: &Path) -> Option<u64> {
    let path = c_path(path).ok()?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return None;
    }
    let stat = unsafe { stat.assume_init() };
    Some(stat.f_bavail.saturating_mul(stat.f_frsize))
}

// All content is written through an anchored directory descriptor. Archive directory modes
// are deferred until the tree is complete, so the private staging tree stays inaccessible.
struct Tree {
    root: File,
    mask: u32,
    directories: BTreeMap<PathBuf, Option<(i64, i64)>>,
}
impl Tree {
    fn new(path: &Path) -> io::Result<Self> {
        let root = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        let mask = fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|text| {
                text.lines().find_map(|line| {
                    line.strip_prefix("Umask:")
                        .and_then(|mask| u32::from_str_radix(mask.trim(), 8).ok())
                })
            })
            .unwrap_or(0o022);
        Ok(Self {
            root,
            mask,
            directories: BTreeMap::new(),
        })
    }
    fn resolved(&mut self, path: &Path, create: bool, follow_final: bool) -> io::Result<PathBuf> {
        let mut queue: VecDeque<_> = path
            .components()
            .filter_map(|component| match component {
                Component::Normal(name) => Some(name.to_owned()),
                Component::CurDir => None,
                _ => Some(OsString::from("..")),
            })
            .collect();
        let mut stack: Vec<OsString> = Vec::new();
        let mut links = 0;
        while let Some(component) = queue.pop_front() {
            if component == ".." {
                if stack.pop().is_none() {
                    return Err(io::Error::other("unsafe archive link"));
                }
                continue;
            }
            let parent = self.open_dir(&stack)?;
            let name = CString::new(component.as_bytes())
                .map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?;
            let mut st = std::mem::MaybeUninit::<libc::stat>::uninit();
            let status = unsafe {
                libc::fstatat(
                    parent.as_raw_fd(),
                    name.as_ptr(),
                    st.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            let final_part = queue.is_empty();
            if status == 0
                && unsafe { st.assume_init() }.st_mode & libc::S_IFMT == libc::S_IFLNK
                && (!final_part || follow_final)
            {
                links += 1;
                if links > 40 {
                    return Err(io::Error::other("unsafe archive link"));
                }
                let mut bytes = vec![0_u8; 4096];
                let n = unsafe {
                    libc::readlinkat(
                        parent.as_raw_fd(),
                        name.as_ptr(),
                        bytes.as_mut_ptr().cast(),
                        bytes.len(),
                    )
                };
                if n < 0 {
                    return Err(io::Error::last_os_error());
                }
                bytes.truncate(n as usize);
                let target = PathBuf::from(OsString::from_vec(bytes));
                if target.is_absolute() {
                    return Err(io::Error::other("unsafe archive link"));
                }
                let mut next: VecDeque<_> = target
                    .components()
                    .filter_map(|component| match component {
                        Component::Normal(name) => Some(name.to_owned()),
                        Component::ParentDir => Some(OsString::from("..")),
                        Component::CurDir => None,
                        _ => Some(OsString::from("..")),
                    })
                    .collect();
                next.append(&mut queue);
                queue = next;
                continue;
            }
            if !final_part {
                if status < 0 {
                    let error = io::Error::last_os_error();
                    if error.kind() != io::ErrorKind::NotFound || !create {
                        return Err(error);
                    }
                    if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
                        return Err(io::Error::last_os_error());
                    }
                    let mut relative = PathBuf::new();
                    for part in &stack {
                        relative.push(part);
                    }
                    relative.push(&component);
                    self.directories.entry(relative).or_insert(None);
                } else if unsafe { st.assume_init() }.st_mode & libc::S_IFMT != libc::S_IFDIR {
                    return Err(io::Error::from_raw_os_error(libc::ENOTDIR));
                }
            }
            stack.push(component);
        }
        Ok(stack.into_iter().collect())
    }
    fn open_dir(&self, parts: &[OsString]) -> io::Result<File> {
        let mut directory = self.root.try_clone()?;
        for part in parts {
            let name = CString::new(part.as_bytes())
                .map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?;
            let fd = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            directory = unsafe { File::from_raw_fd(fd) };
        }
        Ok(directory)
    }
    fn parent(&self, path: &Path) -> io::Result<(File, CString)> {
        let parts: Vec<_> = path
            .parent()
            .unwrap_or(Path::new(""))
            .components()
            .filter_map(|component| {
                if let Component::Normal(name) = component {
                    Some(name.to_owned())
                } else {
                    None
                }
            })
            .collect();
        Ok((
            self.open_dir(&parts)?,
            CString::new(path.file_name().unwrap_or(OsStr::new("")).as_bytes())
                .map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?,
        ))
    }
    fn directory(&mut self, path: &Path, time: Option<(i64, i64)>) -> io::Result<()> {
        let path = self.resolved(path, true, true)?;
        if path.as_os_str().is_empty() {
            self.directories.insert(path, time);
            return Ok(());
        }
        let (parent, name) = self.parent(&path)?;
        if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } != 0
            && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists
        {
            return Err(io::Error::last_os_error());
        }
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        drop(unsafe { File::from_raw_fd(fd) });
        self.directories.insert(path, time);
        Ok(())
    }
    fn regular(&mut self, path: &Path) -> io::Result<File> {
        let path = self.resolved(path, true, true)?;
        if path.as_os_str().is_empty() {
            return Err(io::Error::from_raw_os_error(libc::EISDIR));
        }
        let (parent, name) = self.parent(&path)?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_WRONLY
                    | libc::O_CREAT
                    | libc::O_TRUNC
                    | libc::O_NOFOLLOW
                    | libc::O_NONBLOCK
                    | libc::O_CLOEXEC,
                0o666,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        if !file.metadata()?.is_file() {
            return Err(io::Error::from_raw_os_error(libc::EINVAL));
        }
        Ok(file)
    }
    fn symlink(&mut self, path: &Path, target: &[u8]) -> io::Result<()> {
        let path = self.resolved(path, true, false)?;
        let target_path = normalized_link(&PathBuf::from(OsString::from_vec(target.to_vec())));
        let joined = path.parent().unwrap_or(Path::new("")).join(&target_path);
        let joined = normalized(&joined).ok_or_else(|| io::Error::other("unsafe archive link"))?;
        match self.resolved(&joined, false, true) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let (parent, name) = self.parent(&path)?;
        let target = c_path(&target_path)?;
        if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0) } != 0
            && io::Error::last_os_error().kind() != io::ErrorKind::NotFound
        {
            return Err(io::Error::last_os_error());
        }
        if unsafe { libc::symlinkat(target.as_ptr(), parent.as_raw_fd(), name.as_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    fn hardlink(&mut self, path: &Path, target: &[u8]) -> io::Result<()> {
        let path = self.resolved(path, true, false)?;
        let target = self.resolved(
            &PathBuf::from(OsString::from_vec(target.to_vec())),
            false,
            true,
        )?;
        let (parent, name) = self.parent(&path)?;
        let (source_parent, source_name) = self.parent(&target)?;
        let mut st = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                source_parent.as_raw_fd(),
                source_name.as_ptr(),
                st.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        if unsafe { st.assume_init() }.st_mode & libc::S_IFMT != libc::S_IFREG {
            return Err(io::Error::other("unsafe archive link"));
        }
        if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0) } != 0
            && io::Error::last_os_error().kind() != io::ErrorKind::NotFound
        {
            return Err(io::Error::last_os_error());
        }
        if unsafe {
            libc::linkat(
                source_parent.as_raw_fd(),
                source_name.as_ptr(),
                parent.as_raw_fd(),
                name.as_ptr(),
                0,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    fn finish(&mut self) -> io::Result<()> {
        let mut directories: Vec<_> = self
            .directories
            .iter()
            .map(|(path, time)| (path.clone(), *time))
            .collect();
        directories.sort_by(|a, b| b.0.cmp(&a.0));
        for (path, time) in directories {
            let parts: Vec<_> = path
                .components()
                .filter_map(|part| {
                    if let Component::Normal(name) = part {
                        Some(name.to_owned())
                    } else {
                        None
                    }
                })
                .collect();
            let directory = self.open_dir(&parts)?;
            if unsafe { libc::fchmod(directory.as_raw_fd(), 0o777 & !self.mask) } != 0 {
                return Err(io::Error::last_os_error());
            }
            if let Some((seconds, nsec)) = time {
                set_time(&directory, seconds, nsec)?;
            }
        }
        if unsafe { libc::fchmod(self.root.as_raw_fd(), 0o755 & !self.mask) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

fn set_time(file: &File, seconds: i64, nsec: i64) -> io::Result<()> {
    let times = [
        libc::timespec {
            tv_sec: seconds,
            tv_nsec: nsec.clamp(0, 999_999_999),
        },
        libc::timespec {
            tv_sec: seconds,
            tv_nsec: nsec.clamp(0, 999_999_999),
        },
    ];
    if unsafe { libc::futimens(file.as_raw_fd(), times.as_ptr()) } != 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
fn extraction_io(error: io::Error, path: &Path, lang: Lang) -> ApiError {
    if error.to_string() == "unsafe archive link" {
        unsafe_link(lang)
    } else {
        io_error(error, path, lang)
    }
}
fn tar_metadata(file: &File, entry: &Entry) -> io::Result<()> {
    let mut mode = entry.mode & 0o755;
    if mode & 0o100 == 0 {
        mode &= !0o111;
    }
    mode |= 0o600;
    if unsafe { libc::fchmod(file.as_raw_fd(), mode) } != 0 {
        return Err(io::Error::last_os_error());
    }
    set_time(file, entry.mtime, entry.nsec)
}
fn hardlink_metadata(tree: &mut Tree, path: &Path, entry: &Entry) -> io::Result<()> {
    let path = tree.resolved(path, false, true)?;
    let (parent, name) = tree.parent(&path)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    tar_metadata(&file, entry)
}
fn extract_tree(
    reader: &mut Reader,
    root: &Path,
    kind: Kind,
    progress: &mut Progress<'_>,
    archive: &Path,
) -> Result<(), ApiError> {
    let lang = reader.lang;
    let mut tree = Tree::new(root).map_err(|error| io_error(error, archive, lang))?;
    let mut pending = Vec::new();
    let mut buffer = vec![0_u8; CHUNK];
    while let Some(entry) = reader.next()? {
        validate(&entry, kind, lang)?;
        progress.current = if kind == Kind::Zip {
            entry.name.clone()
        } else {
            String::new()
        };
        let path = normalized(Path::new(&entry.name)).ok_or_else(|| unsafe_path(lang))?;
        let directory = if kind == Kind::Zip {
            entry.name.ends_with('/')
        } else {
            entry.kind == libc::S_IFDIR
        };
        if directory {
            tree.directory(
                &path,
                if kind == Kind::Tar {
                    Some((entry.mtime, entry.nsec))
                } else {
                    None
                },
            )
            .map_err(|error| extraction_io(error, archive, lang))?;
            reader.skip()?;
        } else if kind == Kind::Tar && entry.symlink.is_some() {
            tree.symlink(&path, entry.symlink.as_ref().unwrap())
                .map_err(|error| extraction_io(error, archive, lang))?;
            reader.skip()?;
        } else if kind == Kind::Tar && entry.hardlink.is_some() {
            match tree.hardlink(&path, entry.hardlink.as_ref().unwrap()) {
                Ok(()) => hardlink_metadata(&mut tree, &path, &entry)
                    .map_err(|error| extraction_io(error, archive, lang))?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    pending.push((path, entry.clone()))
                }
                Err(error) => return Err(extraction_io(error, archive, lang)),
            }
            reader.skip()?;
        } else if kind == Kind::Zip || entry.kind == libc::S_IFREG || entry.kind == 0 {
            let mut output = tree
                .regular(&path)
                .map_err(|error| extraction_io(error, archive, lang))?;
            let mut count = 0_u64;
            loop {
                let n = reader.data(&mut buffer)?;
                if n == 0 {
                    break;
                }
                output
                    .write_all(&buffer[..n])
                    .map_err(|error| io_error(error, archive, lang))?;
                count += n as u64;
            }
            if count != entry.size {
                return Err(bad_archive(
                    &crate::i18n::tr(lang, "files.extractedSizeWrong"),
                    lang,
                ));
            }
            if kind == Kind::Tar {
                tar_metadata(&output, &entry).map_err(|error| io_error(error, archive, lang))?;
            }
        } else {
            return Err(unsafe_device(lang));
        }
        if kind == Kind::Zip {
            progress.add(entry.size);
        }
    }
    while !pending.is_empty() {
        let mut next = Vec::new();
        let before = pending.len();
        for (path, entry) in pending {
            match tree.hardlink(&path, entry.hardlink.as_ref().unwrap()) {
                Ok(()) => hardlink_metadata(&mut tree, &path, &entry)
                    .map_err(|error| extraction_io(error, archive, lang))?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => next.push((path, entry)),
                Err(error) => return Err(extraction_io(error, archive, lang)),
            }
        }
        if next.len() == before {
            return Err(bad_archive(
                &crate::i18n::tr(lang, "files.hardLinkMissing"),
                lang,
            ));
        }
        pending = next;
    }
    tree.finish()
        .map_err(|error| io_error(error, archive, lang))?;
    if kind == Kind::Tar {
        progress.add(progress.total.unwrap_or(0));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("win11-archive-{}", nonce()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn context(&self) -> Context {
            Context::new(self.0.clone(), self.0.join("Desktop"), "nl".into(), false)
        }
        fn output(&self) -> PathBuf {
            let path = self.0.join("output");
            fs::create_dir_all(&path).unwrap();
            path
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    struct Fixture<'a> {
        name: &'a str,
        kind: u32,
        mode: u32,
        data: &'a [u8],
        link: Option<&'a str>,
        hard: bool,
    }
    impl<'a> Fixture<'a> {
        fn file(name: &'a str, data: &'a [u8]) -> Self {
            Self {
                name,
                kind: libc::S_IFREG,
                mode: 0o644,
                data,
                link: None,
                hard: false,
            }
        }
        fn directory(name: &'a str) -> Self {
            Self {
                name,
                kind: libc::S_IFDIR,
                mode: 0o000,
                data: b"",
                link: None,
                hard: false,
            }
        }
        fn link(name: &'a str, target: &'a str, hard: bool) -> Self {
            Self {
                name,
                kind: if hard { libc::S_IFREG } else { libc::S_IFLNK },
                mode: 0o777,
                data: b"",
                link: Some(target),
                hard,
            }
        }
    }
    fn fixture(path: &Path, zip: bool, filter: &str, entries: &[Fixture<'_>]) {
        let lib = ffi::library().unwrap();
        let file = File::create(path).unwrap();
        let handle = unsafe { (lib.archive_write_new)() };
        let mut writer = ArchiveWriter {
            lib,
            handle,
            _file: file,
            closed: false,
            lang: Lang::Nl,
        };
        let format = unsafe {
            if zip {
                (lib.archive_write_set_format_zip)(handle)
            } else {
                (lib.archive_write_set_format_pax_restricted)(handle)
            }
        };
        writer.check(format).unwrap();
        if zip {
            writer
                .check(unsafe {
                    (lib.archive_write_set_options)(handle, c"zip:compression=store".as_ptr())
                })
                .unwrap();
        }
        let status = unsafe {
            match filter {
                "gz" => (lib.archive_write_add_filter_gzip)(handle),
                "bz2" => (lib.archive_write_add_filter_bzip2)(handle),
                "xz" => (lib.archive_write_add_filter_xz)(handle),
                "zst" => (lib.archive_write_add_filter_zstd)(handle),
                _ => OK,
            }
        };
        writer.check(status).unwrap();
        writer
            .check(unsafe { (lib.archive_write_set_bytes_per_block)(handle, 0) })
            .unwrap();
        writer
            .check(unsafe { (lib.archive_write_open_fd)(handle, writer._file.as_raw_fd()) })
            .unwrap();
        for value in entries {
            let entry = unsafe { (lib.archive_entry_new)() };
            let name = CString::new(value.name).unwrap();
            let link = value.link.map(|value| CString::new(value).unwrap());
            unsafe {
                (lib.archive_entry_set_pathname_utf8)(entry, name.as_ptr());
                (lib.archive_entry_set_mode)(entry, value.kind | value.mode);
                (lib.archive_entry_set_mtime)(entry, 1_600_000_000, 0);
                (lib.archive_entry_set_size)(entry, value.data.len() as i64);
                if let Some(link) = &link {
                    if value.hard {
                        (lib.archive_entry_set_hardlink_utf8)(entry, link.as_ptr());
                    } else {
                        (lib.archive_entry_set_symlink_utf8)(entry, link.as_ptr());
                    }
                }
            }
            let status = unsafe { (lib.archive_write_header)(handle, entry) };
            unsafe {
                (lib.archive_entry_free)(entry);
            }
            writer.check(status).unwrap();
            writer.data(value.data).unwrap();
        }
        writer.finish().unwrap();
    }
    fn extracted(path: &Path, output: &Path, ctx: &Context) -> Result<PathBuf, ApiError> {
        extract(
            &json!({"path":path,"dest":output,"jobId":"archive-test"}),
            ctx,
        )
        .map(|result| PathBuf::from(result["path"].as_str().unwrap()))
    }
    fn no_partial(output: &Path) {
        assert!(fs::read_dir(output).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".partial")
        }));
    }

    #[test]
    fn zip_round_trip_follows_file_links_skips_directory_links_and_keeps_conflicts() {
        let tmp = Temp::new();
        let source = tmp.0.join("source");
        fs::create_dir(&source).unwrap();
        fs::create_dir(source.join("empty")).unwrap();
        fs::write(source.join("file.txt"), b"archive contents").unwrap();
        fs::set_permissions(source.join("file.txt"), fs::Permissions::from_mode(0o400)).unwrap();
        symlink("file.txt", source.join("file-link")).unwrap();
        symlink("empty", source.join("directory-link")).unwrap();
        symlink("missing", source.join("broken-link")).unwrap();
        let fifo = c_path(&source.join("pipe")).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        let output = tmp.output();
        fs::write(output.join("source.zip"), b"existing").unwrap();
        let result = compress(&json!({"paths":[source],"dest":output}), &tmp.context()).unwrap();
        let archive = PathBuf::from(result["path"].as_str().unwrap());
        assert_eq!(archive.file_name().unwrap(), "source (2).zip");
        assert_eq!(fs::read(output.join("source.zip")).unwrap(), b"existing");
        let destination = extracted(&archive, &output, &tmp.context()).unwrap();
        let restored = destination.join("source");
        assert_eq!(
            fs::read(restored.join("file.txt")).unwrap(),
            b"archive contents"
        );
        assert_eq!(
            fs::read(restored.join("file-link")).unwrap(),
            b"archive contents"
        );
        assert!(
            fs::symlink_metadata(restored.join("file-link"))
                .unwrap()
                .is_file()
        );
        assert!(restored.join("empty").is_dir());
        for name in ["directory-link", "broken-link", "pipe"] {
            assert!(!restored.join(name).exists());
        }
        assert_ne!(
            fs::metadata(restored.join("file.txt"))
                .unwrap()
                .permissions()
                .mode()
                & 0o200,
            0
        );
        assert_ne!(
            fs::metadata(restored.join("file.txt")).unwrap().mtime(),
            1_600_000_000
        );
        no_partial(&output);
    }

    #[test]
    fn tar_filters_safe_links_and_python_data_metadata() {
        let tmp = Temp::new();
        let output = tmp.output();
        for (extension, filter) in [
            ("tar", ""),
            ("tar.gz", "gz"),
            ("tar.bz2", "bz2"),
            ("tar.xz", "xz"),
            ("tar.zst", "zst"),
        ] {
            let path = tmp.0.join(format!("fixture.{extension}"));
            let mut file = Fixture::file("directory/file", b"safe contents");
            file.mode = 0o4777;
            fixture(
                &path,
                false,
                filter,
                &[
                    Fixture::directory("directory"),
                    file,
                    Fixture::link("directory/symlink", "file", false),
                    Fixture::link("hardlink", "directory/file", true),
                ],
            );
            let result = extracted(&path, &output, &tmp.context()).unwrap();
            assert_eq!(fs::read(result.join("hardlink")).unwrap(), b"safe contents");
            assert_eq!(
                fs::read_link(result.join("directory/symlink")).unwrap(),
                Path::new("file")
            );
            let regular = fs::metadata(result.join("directory/file")).unwrap();
            assert_eq!(
                regular.ino(),
                fs::metadata(result.join("hardlink")).unwrap().ino()
            );
            // The hardlink's metadata applies last, exactly like tarfile's extraction order.
            assert_eq!(regular.mode() & 0o7777, 0o755);
            assert_eq!(regular.mtime(), 1_600_000_000);
            let directory = fs::metadata(result.join("directory")).unwrap();
            assert_ne!(directory.mode() & 0o700, 0);
            assert_eq!(directory.mtime(), 1_600_000_000);
            no_partial(&output);
        }
    }

    #[test]
    fn rejects_unsafe_members_links_devices_and_symlink_alias_escape() {
        let tmp = Temp::new();
        let output = tmp.output();
        let sentinel = tmp.0.join("outside");
        fs::write(&sentinel, b"untouched").unwrap();
        for zip in [false, true] {
            for name in [
                "../outside",
                "/outside",
                "C:/outside",
                "nested\\..\\outside",
            ] {
                let path = tmp.0.join(format!("unsafe-{}-{}.arc", zip, nonce()));
                fixture(&path, zip, "", &[Fixture::file(name, b"malicious")]);
                assert_eq!(
                    extracted(&path, &output, &tmp.context()).unwrap_err().code,
                    "unsafe_archive"
                );
            }
        }
        for (name, target, hard) in [
            ("link", "../outside", false),
            ("dir/link", "../../outside", false),
            ("link", "/outside", false),
            ("hard", "../outside", true),
        ] {
            let path = tmp.0.join(format!("unsafe-{}.tar", nonce()));
            fixture(&path, false, "", &[Fixture::link(name, target, hard)]);
            assert_eq!(
                extracted(&path, &output, &tmp.context()).unwrap_err().code,
                "unsafe_archive"
            );
        }
        let path = tmp.0.join("device.tar");
        fixture(
            &path,
            false,
            "",
            &[Fixture {
                name: "pipe",
                kind: libc::S_IFIFO,
                mode: 0o600,
                data: b"",
                link: None,
                hard: false,
            }],
        );
        assert_eq!(
            extracted(&path, &output, &tmp.context()).unwrap_err().code,
            "unsafe_archive"
        );
        let path = tmp.0.join("alias.tar");
        fixture(
            &path,
            false,
            "",
            &[
                Fixture::link("alias", ".", false),
                Fixture::link("alias/escape", "../outside", false),
                Fixture::file("alias/escape", b"malicious"),
            ],
        );
        assert_eq!(
            extracted(&path, &output, &tmp.context()).unwrap_err().code,
            "unsafe_archive"
        );
        assert_eq!(fs::read(&sentinel).unwrap(), b"untouched");
        assert_eq!(fs::read_dir(&output).unwrap().count(), 0);
    }

    #[test]
    fn zip_unix_symlink_attributes_remain_regular_binary_data() {
        let tmp = Temp::new();
        let output = tmp.output();
        let path = tmp.0.join("attributes.zip");
        fixture(
            &path,
            true,
            "",
            &[Fixture::file("link", b"arbitrary\0target")],
        );
        let mut bytes = fs::read(&path).unwrap();
        let central = bytes
            .windows(4)
            .position(|bytes| bytes == b"PK\x01\x02")
            .unwrap();
        bytes[central + 38..central + 42]
            .copy_from_slice(&((libc::S_IFLNK | 0o777) << 16).to_le_bytes());
        fs::write(&path, &bytes).unwrap();
        let result = extracted(&path, &output, &tmp.context()).unwrap();
        assert!(fs::symlink_metadata(result.join("link")).unwrap().is_file());
        assert_eq!(fs::read(result.join("link")).unwrap(), b"arbitrary\0target");
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn corrupt_payload_is_transactional_and_empty_archives_are_supported() {
        let tmp = Temp::new();
        let output = tmp.output();
        let path = tmp.0.join("corrupt.zip");
        fixture(&path, true, "", &[Fixture::file("file", b"contents")]);
        fs::create_dir(output.join("corrupt")).unwrap();
        fs::write(output.join("corrupt/sentinel"), b"untouched").unwrap();
        let mut bytes = fs::read(&path).unwrap();
        let local = bytes
            .windows(4)
            .position(|bytes| bytes == b"PK\x03\x04")
            .unwrap();
        let data =
            local + 30 + u16le(&bytes, local + 26) as usize + u16le(&bytes, local + 28) as usize;
        bytes[data] ^= 0x40;
        fs::write(&path, bytes).unwrap();
        assert_eq!(
            extracted(&path, &output, &tmp.context()).unwrap_err().code,
            "bad_archive"
        );
        assert_eq!(
            fs::read(output.join("corrupt/sentinel")).unwrap(),
            b"untouched"
        );
        assert_eq!(fs::read_dir(&output).unwrap().count(), 1);
        for zip in [false, true] {
            let path = tmp.0.join(if zip { "empty.zip" } else { "empty.tar" });
            fixture(&path, zip, "", &[]);
            let result = extracted(&path, &output, &tmp.context()).unwrap();
            assert_eq!(fs::read_dir(result).unwrap().count(), 0);
        }
        let path = tmp.0.join("plain.zip");
        fs::write(&path, b"plain text").unwrap();
        assert_eq!(
            extracted(&path, &output, &tmp.context()).unwrap_err().code,
            "unsupported_archive"
        );
        no_partial(&output);
    }

    #[test]
    fn duplicate_members_overwrite_only_inside_new_directory() {
        let tmp = Temp::new();
        let output = tmp.output();
        let path = tmp.0.join("duplicate.tar.gz");
        fixture(
            &path,
            true,
            "",
            &[
                Fixture::file("nested/file", b"first"),
                Fixture::file("nested/file", b"second"),
            ],
        );
        fs::create_dir(output.join("duplicate")).unwrap();
        fs::write(output.join("duplicate/sentinel"), b"old").unwrap();
        let result = extracted(&path, &output, &tmp.context()).unwrap();
        assert_eq!(result.file_name().unwrap(), "duplicate (2)");
        assert_eq!(fs::read(result.join("nested/file")).unwrap(), b"second");
        assert_eq!(fs::read(output.join("duplicate/sentinel")).unwrap(), b"old");
        no_partial(&output);
    }

    #[test]
    fn publish_keeps_chosen_name_and_retries_like_reference() {
        let tmp = Temp::new();
        let output = tmp.output();
        let initial = output.join("result (2).zip");
        let stage = Staging::new(&output, "result (2).zip", false).unwrap();
        fs::write(&stage.payload, b"new archive").unwrap();
        fs::write(&initial, b"concurrent archive").unwrap();
        let published = stage.publish(&initial, false).unwrap();
        assert_eq!(published.file_name().unwrap(), "result (3).zip");
        assert_eq!(fs::read(initial).unwrap(), b"concurrent archive");
        assert_eq!(fs::read(published).unwrap(), b"new archive");
        assert!(!output.join("result.zip").exists());
    }

    // The libarchive load error (cached as a kind, rendered per call): key + params (C2), en and 3b898e6's nl.
    #[test]
    fn libarchive_load_errors_render_in_the_call_language_with_their_key() {
        let missing = load_error(&ffi::LoadError::Missing, Lang::En);
        assert_eq!(missing.code, "unsupported_archive");
        assert_eq!(
            missing.message,
            "libarchive.so.13 is missing; install the libarchive runtime package."
        );
        assert_eq!(missing.json()["key"], "files.libarchiveMissing");
        assert_eq!(missing.json()["params"], json!({}));
        let symbol = load_error(&ffi::LoadError::Symbol("archive_read_new"), Lang::En);
        assert_eq!(symbol.code, "unsupported_archive");
        assert_eq!(
            symbol.message,
            "The archive library is missing archive_read_new"
        );
        assert_eq!(symbol.json()["key"], "files.libarchiveLacks");
        assert_eq!(symbol.json()["params"], json!({}));
        assert_eq!(
            load_error(&ffi::LoadError::Missing, Lang::Nl).message,
            "libarchive.so.13 ontbreekt; installeer het libarchive-runtimepakket."
        );
        assert_eq!(
            load_error(&ffi::LoadError::Symbol("archive_read_new"), Lang::Nl).message,
            "De archiefbibliotheek mist archive_read_new"
        );
    }
}
