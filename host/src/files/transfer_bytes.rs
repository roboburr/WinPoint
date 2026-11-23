//! Bounded byte transfers between already-open, caller-validated regular files.
//!
//! This module deliberately owns no paths, metadata, progress counters or job
//! controls. The caller must check cancellation and destination ownership before
//! and after each `copy_next` call, account the returned bytes, and perform its
//! usual final source/metadata checks. A successful step writes at most 8 MiB;
//! the buffered fallback writes at most 1 MiB. Files are never reopened, sought,
//! truncated, cloned or flushed here.

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;

const KERNEL_CHUNK_BYTES: usize = 8 * 1024 * 1024;
const BUFFER_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    Kernel,
    Buffered,
    Eof,
    Failed,
}

/// Per-file state; create a fresh engine for every source/destination pair.
#[derive(Debug)]
pub(super) struct CopyEngine {
    mode: Mode,
}

/// Bytes actually written by one step. EOF is reported separately from progress.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CopyChunk {
    pub bytes_written: usize,
    pub eof: bool,
}

/// A terminal error, including any bytes written before a buffered write failed.
///
/// Refresh destination metadata and account `bytes_written` before applying the
/// caller's usual failure/cleanup policy. Do not retry this file with this engine:
/// a buffered read can advance the source beyond the successfully written bytes.
#[derive(Debug)]
pub(super) struct CopyError {
    pub error: io::Error,
    pub bytes_written: usize,
}

impl std::fmt::Display for CopyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for CopyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl Default for CopyEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl CopyEngine {
    pub fn new() -> Self {
        Self { mode: Mode::Kernel }
    }

    /// Transfer one bounded chunk at the files' current offsets.
    ///
    /// `buffer` belongs to the job and may be reused across all of its files. It
    /// is allocated/grown lazily only when a buffered transfer is needed. Neither
    /// descriptor may be used concurrently elsewhere; the caller must keep them
    /// bound to the same regular files for this engine's lifetime. The output
    /// must be writable without `O_APPEND`, as with a newly created staging file.
    ///
    /// `before_write` revalidates output ownership immediately before each kernel
    /// copy attempt and after a buffered read, before any corresponding write. It
    /// may also check cancellation. A guard error is terminal and writes no bytes
    /// in that step. The caller can borrow both files immutably in this closure.
    ///
    /// EINTR retries preserve offsets and partial-write accounting. Unsupported
    /// kernel copies fall back at the current offsets, including after earlier
    /// successful/short kernel steps. Other errors are returned without fallback.
    pub fn copy_next(
        &mut self,
        input: &File,
        output: &File,
        buffer: &mut Vec<u8>,
        mut before_write: impl FnMut() -> io::Result<()>,
    ) -> Result<CopyChunk, CopyError> {
        self.copy_next_guarded(input, output, buffer, &mut SystemIo, &mut before_write)
    }

    // This private seam permits deterministic syscall/error tests without global
    // switches or changes to the public production API.
    fn copy_next_guarded(
        &mut self,
        input: &File,
        output: &File,
        buffer: &mut Vec<u8>,
        operations: &mut impl CopyIo,
        before_write: &mut impl FnMut() -> io::Result<()>,
    ) -> Result<CopyChunk, CopyError> {
        let result = self.copy_next_inner(input, output, buffer, operations, before_write);
        if result.is_err() {
            self.mode = Mode::Failed;
        }
        result
    }

    fn copy_next_inner(
        &mut self,
        input: &File,
        output: &File,
        buffer: &mut Vec<u8>,
        operations: &mut impl CopyIo,
        before_write: &mut impl FnMut() -> io::Result<()>,
    ) -> Result<CopyChunk, CopyError> {
        match self.mode {
            Mode::Eof => {
                return Ok(CopyChunk {
                    bytes_written: 0,
                    eof: true,
                });
            }
            Mode::Failed => {
                return Err(CopyError {
                    error: io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "cannot resume a failed byte transfer",
                    ),
                    bytes_written: 0,
                });
            }
            Mode::Kernel | Mode::Buffered => {}
        }

        while self.mode == Mode::Kernel {
            before_write().map_err(|error| CopyError {
                error,
                bytes_written: 0,
            })?;
            match operations.copy_file_range(input, output, KERNEL_CHUNK_BYTES) {
                Ok(0) => {
                    // Some filesystems have returned zero despite unread source
                    // bytes. Confirm EOF with read(), rather than silently finish
                    // a truncated copy. No offsets are reset in this transition.
                    self.mode = Mode::Buffered;
                }
                Ok(bytes_written) => {
                    return Ok(CopyChunk {
                        bytes_written,
                        eof: false,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if unsupported_copy(&error) => self.mode = Mode::Buffered,
                Err(error) => {
                    return Err(CopyError {
                        error,
                        bytes_written: 0,
                    });
                }
            }
        }

        if buffer.len() < BUFFER_BYTES {
            buffer
                .try_reserve_exact(BUFFER_BYTES - buffer.len())
                .map_err(|_| CopyError {
                    error: io::Error::new(
                        io::ErrorKind::OutOfMemory,
                        "cannot allocate file transfer buffer",
                    ),
                    bytes_written: 0,
                })?;
            buffer.resize(BUFFER_BYTES, 0);
        }

        let bytes_read = loop {
            match operations.read(input, &mut buffer[..BUFFER_BYTES]) {
                Ok(bytes_read) => break bytes_read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    return Err(CopyError {
                        error,
                        bytes_written: 0,
                    });
                }
            }
        };
        if bytes_read == 0 {
            self.mode = Mode::Eof;
            return Ok(CopyChunk {
                bytes_written: 0,
                eof: true,
            });
        }

        let mut bytes_written = 0;
        before_write().map_err(|error| CopyError {
            error,
            bytes_written: 0,
        })?;
        while bytes_written < bytes_read {
            match operations.write(output, &buffer[bytes_written..bytes_read]) {
                Ok(0) => {
                    return Err(CopyError {
                        error: io::Error::new(
                            io::ErrorKind::WriteZero,
                            "file transfer made no write progress",
                        ),
                        bytes_written,
                    });
                }
                Ok(written) => bytes_written += written,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    return Err(CopyError {
                        error,
                        bytes_written,
                    });
                }
            }
        }
        Ok(CopyChunk {
            bytes_written,
            eof: false,
        })
    }

    #[cfg(test)]
    fn copy_next_using(
        &mut self,
        input: &File,
        output: &File,
        buffer: &mut Vec<u8>,
        operations: &mut impl CopyIo,
    ) -> Result<CopyChunk, CopyError> {
        self.copy_next_guarded(input, output, buffer, operations, &mut || Ok(()))
    }
}

fn unsupported_copy(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::ENOSYS | libc::EXDEV | libc::EOPNOTSUPP | libc::EINVAL | libc::EPERM)
    )
}

trait CopyIo {
    fn copy_file_range(&mut self, input: &File, output: &File, length: usize) -> io::Result<usize>;

    fn read(&mut self, mut input: &File, buffer: &mut [u8]) -> io::Result<usize> {
        input.read(buffer)
    }

    fn write(&mut self, mut output: &File, buffer: &[u8]) -> io::Result<usize> {
        output.write(buffer)
    }
}

struct SystemIo;

impl CopyIo for SystemIo {
    fn copy_file_range(&mut self, input: &File, output: &File, length: usize) -> io::Result<usize> {
        // SAFETY: File owns each live descriptor for the call. Null offset
        // pointers request the kernel to advance the existing descriptor offsets;
        // there are no user buffers/pointers and flags must be zero.
        let copied = unsafe {
            libc::copy_file_range(
                input.as_raw_fd(),
                std::ptr::null_mut(),
                output.as_raw_fd(),
                std::ptr::null_mut(),
                length,
                0,
            )
        };
        if copied < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(copied as usize)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::fs::{self, OpenOptions};
    use std::io::{Seek, SeekFrom};
    use std::os::unix::fs::DirBuilderExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        directory: PathBuf,
        input: File,
        output: File,
    }

    impl Fixture {
        fn new(bytes: &[u8]) -> Self {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let directory = std::env::temp_dir().join(format!(
                "winpoint-byte-copy-{}-{timestamp}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            fs::DirBuilder::new()
                .mode(0o700)
                .create(&directory)
                .unwrap();
            let input_path = directory.join("source");
            let mut source = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&input_path)
                .unwrap();
            source.write_all(bytes).unwrap();
            drop(source);
            let input = File::open(input_path).unwrap();
            let output = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(directory.join("destination"))
                .unwrap();
            Self {
                directory,
                input,
                output,
            }
        }

        fn output_bytes(&mut self) -> Vec<u8> {
            self.output.seek(SeekFrom::Start(0)).unwrap();
            let mut bytes = Vec::new();
            self.output.read_to_end(&mut bytes).unwrap();
            bytes
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    enum RangeAction {
        Copy(usize),
        Error(i32),
        Zero,
    }

    enum WriteAction {
        Prefix(usize),
        Error(i32),
        Zero,
    }

    #[derive(Default)]
    struct MockIo {
        ranges: VecDeque<RangeAction>,
        read_errors: VecDeque<i32>,
        writes: VecDeque<WriteAction>,
        range_lengths: Vec<usize>,
        reads: usize,
    }

    impl MockIo {
        fn with_ranges(actions: impl IntoIterator<Item = RangeAction>) -> Self {
            Self {
                ranges: actions.into_iter().collect(),
                ..Self::default()
            }
        }
    }

    impl CopyIo for MockIo {
        fn copy_file_range(
            &mut self,
            mut input: &File,
            mut output: &File,
            length: usize,
        ) -> io::Result<usize> {
            self.range_lengths.push(length);
            match self
                .ranges
                .pop_front()
                .unwrap_or(RangeAction::Copy(usize::MAX))
            {
                RangeAction::Copy(maximum) => {
                    let remaining = input
                        .metadata()?
                        .len()
                        .saturating_sub(input.stream_position()?);
                    let length = length.min(maximum).min(usize::try_from(remaining).unwrap());
                    let mut buffer = vec![0; length];
                    let read = input.read(&mut buffer)?;
                    output.write_all(&buffer[..read])?;
                    Ok(read)
                }
                RangeAction::Error(errno) => Err(io::Error::from_raw_os_error(errno)),
                RangeAction::Zero => Ok(0),
            }
        }

        fn read(&mut self, mut input: &File, buffer: &mut [u8]) -> io::Result<usize> {
            self.reads += 1;
            if let Some(errno) = self.read_errors.pop_front() {
                return Err(io::Error::from_raw_os_error(errno));
            }
            input.read(buffer)
        }

        fn write(&mut self, mut output: &File, buffer: &[u8]) -> io::Result<usize> {
            match self.writes.pop_front() {
                Some(WriteAction::Prefix(maximum)) => {
                    output.write(&buffer[..buffer.len().min(maximum)])
                }
                Some(WriteAction::Error(errno)) => Err(io::Error::from_raw_os_error(errno)),
                Some(WriteAction::Zero) => Ok(0),
                None => output.write(buffer),
            }
        }
    }

    fn finish(
        engine: &mut CopyEngine,
        fixture: &mut Fixture,
        buffer: &mut Vec<u8>,
        operations: &mut impl CopyIo,
    ) -> usize {
        let mut total = 0;
        loop {
            let chunk = engine
                .copy_next_using(&mut fixture.input, &mut fixture.output, buffer, operations)
                .unwrap();
            total += chunk.bytes_written;
            if chunk.eof {
                return total;
            }
        }
    }

    #[test]
    fn real_descriptors_copy_bytes_and_report_eof() {
        let bytes: Vec<u8> = (0..BUFFER_BYTES + 37)
            .map(|index| (index % 251) as u8)
            .collect();
        let mut fixture = Fixture::new(&bytes);
        let mut engine = CopyEngine::new();
        let mut buffer = Vec::new();
        let mut total = 0;
        loop {
            let chunk = engine
                .copy_next(&fixture.input, &fixture.output, &mut buffer, || Ok(()))
                .unwrap();
            total += chunk.bytes_written;
            if chunk.eof {
                break;
            }
        }
        assert_eq!(total, bytes.len());
        assert_eq!(fixture.output_bytes(), bytes);
    }

    #[test]
    fn short_kernel_steps_preserve_offsets_when_falling_back() {
        let bytes = b"no lost or duplicated bytes at the backend boundary";
        let mut fixture = Fixture::new(bytes);
        let mut engine = CopyEngine::new();
        let mut buffer = Vec::new();
        let mut operations =
            MockIo::with_ranges([RangeAction::Copy(7), RangeAction::Error(libc::EXDEV)]);
        let first = engine
            .copy_next_using(
                &mut fixture.input,
                &mut fixture.output,
                &mut buffer,
                &mut operations,
            )
            .unwrap();
        assert_eq!(
            first,
            CopyChunk {
                bytes_written: 7,
                eof: false
            }
        );
        assert!(
            buffer.is_empty(),
            "kernel steps must not allocate the fallback buffer"
        );
        let remainder = finish(&mut engine, &mut fixture, &mut buffer, &mut operations);
        assert_eq!(first.bytes_written + remainder, bytes.len());
        assert_eq!(fixture.output_bytes(), bytes);
        assert_eq!(operations.range_lengths, vec![KERNEL_CHUNK_BYTES; 2]);
    }

    #[test]
    fn every_unsupported_errno_falls_back_once() {
        for errno in [
            libc::ENOSYS,
            libc::EXDEV,
            libc::EOPNOTSUPP,
            libc::EINVAL,
            libc::EPERM,
        ] {
            let mut fixture = Fixture::new(b"portable copy");
            let mut operations = MockIo::with_ranges([RangeAction::Error(errno)]);
            let mut engine = CopyEngine::new();
            let total = finish(&mut engine, &mut fixture, &mut Vec::new(), &mut operations);
            assert_eq!(total, 13);
            assert_eq!(fixture.output_bytes(), b"portable copy");
            assert_eq!(operations.range_lengths.len(), 1);
        }
    }

    #[test]
    fn interrupted_and_short_kernel_steps_are_not_eof() {
        let mut fixture = Fixture::new(b"abcdefghi");
        let mut operations = MockIo::with_ranges([
            RangeAction::Error(libc::EINTR),
            RangeAction::Copy(2),
            RangeAction::Copy(3),
        ]);
        let mut engine = CopyEngine::new();
        let total = finish(&mut engine, &mut fixture, &mut Vec::new(), &mut operations);
        assert_eq!(total, 9);
        assert_eq!(fixture.output_bytes(), b"abcdefghi");
        assert!(
            operations
                .range_lengths
                .iter()
                .all(|length| *length == KERNEL_CHUNK_BYTES)
        );
    }

    #[test]
    fn zero_kernel_result_checks_for_unread_source_bytes() {
        let mut fixture = Fixture::new(b"not actually EOF");
        let mut operations = MockIo::with_ranges([RangeAction::Zero]);
        let mut engine = CopyEngine::new();
        let total = finish(&mut engine, &mut fixture, &mut Vec::new(), &mut operations);
        assert_eq!(total, 16);
        assert_eq!(fixture.output_bytes(), b"not actually EOF");
        assert_eq!(operations.range_lengths.len(), 1);
    }

    #[test]
    fn fatal_kernel_error_does_not_read_or_retry_the_file() {
        let mut fixture = Fixture::new(b"untouched");
        let mut operations = MockIo::with_ranges([RangeAction::Error(libc::EIO)]);
        let mut engine = CopyEngine::new();
        let mut buffer = Vec::new();
        let error = engine
            .copy_next_using(
                &mut fixture.input,
                &mut fixture.output,
                &mut buffer,
                &mut operations,
            )
            .unwrap_err();
        assert_eq!(error.error.raw_os_error(), Some(libc::EIO));
        assert_eq!(error.bytes_written, 0);
        assert_eq!(operations.reads, 0);
        assert!(buffer.is_empty());
        let error = engine
            .copy_next_using(
                &mut fixture.input,
                &mut fixture.output,
                &mut buffer,
                &mut operations,
            )
            .unwrap_err();
        assert_eq!(error.error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(operations.range_lengths.len(), 1);
        assert!(fixture.output_bytes().is_empty());
    }

    #[test]
    fn buffered_read_and_write_retry_eintr_and_report_partial_error() {
        let mut fixture = Fixture::new(b"abcdefghi");
        let mut operations = MockIo::with_ranges([RangeAction::Error(libc::ENOSYS)]);
        operations.read_errors.push_back(libc::EINTR);
        operations.writes.extend([
            WriteAction::Prefix(3),
            WriteAction::Error(libc::EINTR),
            WriteAction::Error(libc::ENOSPC),
        ]);
        let mut engine = CopyEngine::new();
        let error = engine
            .copy_next_using(
                &mut fixture.input,
                &mut fixture.output,
                &mut Vec::new(),
                &mut operations,
            )
            .unwrap_err();
        assert_eq!(error.error.raw_os_error(), Some(libc::ENOSPC));
        assert_eq!(error.bytes_written, 3);
        assert_eq!(fixture.input.stream_position().unwrap(), 9);
        assert_eq!(fixture.output_bytes(), b"abc");
        assert_eq!(operations.reads, 2);
    }

    #[test]
    fn fallback_error_accounts_only_current_step_after_kernel_progress() {
        let mut fixture = Fixture::new(b"abcdefghijklmnop");
        let mut operations =
            MockIo::with_ranges([RangeAction::Copy(5), RangeAction::Error(libc::EXDEV)]);
        operations
            .writes
            .extend([WriteAction::Prefix(3), WriteAction::Error(libc::ENOSPC)]);
        let mut engine = CopyEngine::new();
        let mut buffer = Vec::new();
        let first = engine
            .copy_next_using(
                &fixture.input,
                &fixture.output,
                &mut buffer,
                &mut operations,
            )
            .unwrap();
        assert_eq!(first.bytes_written, 5);
        assert_eq!((&fixture.input).stream_position().unwrap(), 5);
        assert_eq!((&fixture.output).stream_position().unwrap(), 5);
        let error = engine
            .copy_next_using(
                &fixture.input,
                &fixture.output,
                &mut buffer,
                &mut operations,
            )
            .unwrap_err();
        assert_eq!(error.error.raw_os_error(), Some(libc::ENOSPC));
        assert_eq!(error.bytes_written, 3);
        assert_eq!((&fixture.input).stream_position().unwrap(), 16);
        assert_eq!((&fixture.output).stream_position().unwrap(), 8);
        assert_eq!(fixture.output_bytes(), b"abcdefgh");
    }

    #[test]
    fn ownership_guard_can_borrow_files_and_prevent_kernel_write() {
        let fixture = Fixture::new(b"never written");
        let mut engine = CopyEngine::new();
        let mut operations = MockIo::default();
        let error = engine
            .copy_next_guarded(
                &fixture.input,
                &fixture.output,
                &mut Vec::new(),
                &mut operations,
                &mut || {
                    assert_eq!(fixture.input.metadata()?.len(), 13);
                    assert_eq!(fixture.output.metadata()?.len(), 0);
                    Err(io::Error::from_raw_os_error(libc::EBUSY))
                },
            )
            .unwrap_err();
        assert_eq!(error.error.raw_os_error(), Some(libc::EBUSY));
        assert_eq!(error.bytes_written, 0);
        assert!(operations.range_lengths.is_empty());
        assert_eq!((&fixture.input).stream_position().unwrap(), 0);
        assert_eq!(fixture.output.metadata().unwrap().len(), 0);
    }

    #[test]
    fn guard_runs_after_buffered_read_before_any_write() {
        let fixture = Fixture::new(b"already read but never written");
        let mut engine = CopyEngine::new();
        let mut operations = MockIo::with_ranges([RangeAction::Error(libc::EXDEV)]);
        let mut guard_calls = 0;
        let error = engine
            .copy_next_guarded(
                &fixture.input,
                &fixture.output,
                &mut Vec::new(),
                &mut operations,
                &mut || {
                    guard_calls += 1;
                    assert_eq!(fixture.output.metadata()?.len(), 0);
                    if guard_calls == 1 {
                        assert_eq!((&fixture.input).stream_position()?, 0);
                        Ok(())
                    } else {
                        assert_eq!((&fixture.input).stream_position()?, 30);
                        Err(io::Error::from_raw_os_error(libc::ECANCELED))
                    }
                },
            )
            .unwrap_err();
        assert_eq!(error.error.raw_os_error(), Some(libc::ECANCELED));
        assert_eq!(error.bytes_written, 0);
        assert_eq!(guard_calls, 2);
        assert_eq!(operations.reads, 1);
        assert_eq!(fixture.output.metadata().unwrap().len(), 0);
    }

    #[test]
    fn read_error_after_fallback_writes_nothing() {
        let fixture = Fixture::new(b"unreadable for the mock");
        let mut operations = MockIo::with_ranges([RangeAction::Error(libc::EOPNOTSUPP)]);
        operations.read_errors.push_back(libc::EIO);
        let error = CopyEngine::new()
            .copy_next_using(
                &fixture.input,
                &fixture.output,
                &mut Vec::new(),
                &mut operations,
            )
            .unwrap_err();
        assert_eq!(error.error.raw_os_error(), Some(libc::EIO));
        assert_eq!(error.bytes_written, 0);
        assert_eq!((&fixture.input).stream_position().unwrap(), 0);
        assert_eq!(fixture.output.metadata().unwrap().len(), 0);
    }

    #[test]
    fn zero_write_is_terminal_and_accounts_prior_writes() {
        let mut fixture = Fixture::new(b"abcdef");
        let mut operations = MockIo::with_ranges([RangeAction::Error(libc::ENOSYS)]);
        operations
            .writes
            .extend([WriteAction::Prefix(2), WriteAction::Zero]);
        let error = CopyEngine::new()
            .copy_next_using(
                &mut fixture.input,
                &mut fixture.output,
                &mut Vec::new(),
                &mut operations,
            )
            .unwrap_err();
        assert_eq!(error.error.kind(), io::ErrorKind::WriteZero);
        assert_eq!(error.bytes_written, 2);
        assert_eq!(fixture.output_bytes(), b"ab");
    }

    #[test]
    fn buffered_steps_are_bounded_and_buffer_reuses_allocation_between_files() {
        let bytes = vec![42; BUFFER_BYTES + 1];
        let mut buffer = Vec::new();
        let mut fixture = Fixture::new(&bytes);
        let mut operations = MockIo::with_ranges([RangeAction::Error(libc::EXDEV)]);
        let mut engine = CopyEngine::new();
        let first = engine
            .copy_next_using(
                &mut fixture.input,
                &mut fixture.output,
                &mut buffer,
                &mut operations,
            )
            .unwrap();
        assert_eq!(
            first,
            CopyChunk {
                bytes_written: BUFFER_BYTES,
                eof: false
            }
        );
        let allocation = buffer.as_ptr();
        assert_eq!(
            finish(&mut engine, &mut fixture, &mut buffer, &mut operations),
            1
        );
        assert_eq!(fixture.output_bytes(), bytes);

        let mut another = Fixture::new(b"another file");
        let mut operations = MockIo::with_ranges([RangeAction::Error(libc::EXDEV)]);
        assert_eq!(
            finish(
                &mut CopyEngine::new(),
                &mut another,
                &mut buffer,
                &mut operations
            ),
            12
        );
        assert_eq!(buffer.as_ptr(), allocation);
        assert_eq!(another.output_bytes(), b"another file");
    }

    #[test]
    fn empty_file_eof_is_idempotent() {
        let mut fixture = Fixture::new(b"");
        let mut operations = MockIo::with_ranges([RangeAction::Zero]);
        let mut engine = CopyEngine::new();
        let mut buffer = Vec::new();
        assert_eq!(
            finish(&mut engine, &mut fixture, &mut buffer, &mut operations),
            0
        );
        let calls = (operations.range_lengths.len(), operations.reads);
        let chunk = engine
            .copy_next_using(
                &mut fixture.input,
                &mut fixture.output,
                &mut buffer,
                &mut operations,
            )
            .unwrap();
        assert_eq!(
            chunk,
            CopyChunk {
                bytes_written: 0,
                eof: true
            }
        );
        assert_eq!((operations.range_lengths.len(), operations.reads), calls);
    }
}
