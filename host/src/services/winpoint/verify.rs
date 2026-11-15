//! Is the download WinPoint's release (D-G74)? No signing key: the trust anchor is GitHub, over validated TLS. The
//! releases API answer from api.github.com (fetched again when the job starts, never a cached one) gives the
//! tarball's `digest` ("sha256:<64 hex>"); the release's `.sha256` asset must name the exact tarball with that same
//! hash; the downloaded tarball's SHA-256, computed while it streams in, must equal both. A release without the
//! digest is never downloaded by WinPoint (`noDigest`: the card links to the release page instead).
use std::{
    fs::File,
    io::{self, Read},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

/// A SHA-256 hash.
pub(crate) type Hash = [u8; 32];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum VerifyError {
    /// not exactly `"<64 lowercase hex>  <name>\n"`
    Format,
    /// a well-formed line for another file
    Name,
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    }
}
/// 64 lowercase hex digits as a hash (an uppercase digit is refused: the frozen format is lowercase).
pub(crate) fn parse_hex(text: &str) -> Option<Hash> {
    let bytes = text.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let mut hash = [0u8; 32];
    for (i, pair) in bytes.chunks(2).enumerate() {
        hash[i] = hex_digit(pair[0])? << 4 | hex_digit(pair[1])?;
    }
    Some(hash)
}
pub(crate) fn to_hex(hash: &Hash) -> String {
    hash.iter().map(|b| format!("{b:02x}")).collect()
}
/// GitHub's asset `digest`, `"sha256:<64 lowercase hex>"` (feed.rs keeps only that form).
pub(crate) fn parse_digest(digest: &str) -> Option<Hash> {
    parse_hex(digest.strip_prefix("sha256:")?)
}

/// The `.sha256` asset (frozen from 0.9.0 on, scripts/package.py writes it): exactly
/// `"<64 lowercase hex>  <expected_name>\n"`, nothing before or after.
pub(crate) fn parse_sha256_file(bytes: &[u8], expected_name: &str) -> Result<Hash, VerifyError> {
    let text = std::str::from_utf8(bytes).map_err(|_| VerifyError::Format)?;
    let line = text.strip_suffix('\n').ok_or(VerifyError::Format)?;
    let (hex, name) = line.split_once("  ").ok_or(VerifyError::Format)?;
    let hash = parse_hex(hex).ok_or(VerifyError::Format)?;
    if name.is_empty() || name.contains(['\n', '\r', '/', '\0']) {
        return Err(VerifyError::Format);
    }
    if name != expected_name {
        return Err(VerifyError::Name);
    }
    Ok(hash)
}

/// SHA-256 over a stream (GLib's checksum).
pub(crate) struct Hasher(glib::Checksum);
impl Hasher {
    pub(crate) fn new() -> Self {
        Self(glib::Checksum::new(glib::ChecksumType::Sha256).expect("GLib has SHA-256"))
    }
    pub(crate) fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }
    pub(crate) fn finish(self) -> Hash {
        let mut hash = [0u8; 32];
        let digest = self.0.digest();
        if digest.len() == hash.len() {
            hash.copy_from_slice(&digest);
        }
        hash
    }
}
impl Default for Hasher {
    fn default() -> Self {
        Self::new()
    }
}

/// The SHA-256 of a whole file (Try again rehashes a tarball downloaded before), read in 1 MiB pieces; `stop`
/// (the job's Cancel) ends it between pieces.
pub(crate) fn hash_file(path: &Path, stop: &AtomicBool) -> io::Result<(Hash, u64)> {
    let mut file = File::open(path)?;
    let mut hasher = Hasher::new();
    let mut buffer = vec![0u8; 1 << 20];
    let mut size = 0u64;
    loop {
        if stop.load(Ordering::SeqCst) {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        let n = file.read(&mut buffer)?;
        if n == 0 {
            return Ok((hasher.finish(), size));
        }
        hasher.update(&buffer[..n]);
        size += n as u64;
    }
}
