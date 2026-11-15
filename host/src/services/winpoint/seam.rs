//! The test seam of WinPoint's updater (D-G74a, D-G74), with no hole.
//!
//! * Rust tests never read the environment: they build [`Seam::for_test`] directly.
//! * A release binary (the GNOME harness) reads one variable, `WIN11_WINPOINT_TEST=<dir>` (not a `WIN11_UPDATE_`
//!   name, which the harness's update-seam guard refuses). It is honoured only when the euid is not 0 and the
//!   directory is absolute, ours, mode 0700, holds the update fixtures' marker (`update::paths::MARKER`) and a
//!   `seam.json` of exactly `{"origin":"http://127.0.0.1:<1024-65535>/","running":"<semver>",
//!   "digest":"sha256:<64 lowercase hex>"}`.
//! * A variable that is set but fails a check makes the service `unsupported` for the life of the process: no
//!   network at all, never a fallback to GitHub.
//! * In test mode `api.github.com` is never contacted: the feed and the release pages are on the origin, every
//!   redirect stays there, `running` replaces the version this host compares with, and the first automatic check
//!   comes 5 s after the owner starts (schedule.rs), so a harness scenario sees the notification.
//!
//! Why this is no hole: the variable can only point the updater at loopback, and whoever can set it already runs
//! code as the user. But a port of 1024 or more on 127.0.0.1 is anyone's: another local account can listen there
//! before the harness's fake GitHub does and serve a list, a digest, a `.sha256` and a tarball of its own, all of
//! which agree. Since D-G74 the job runs the tarball's installer after a click, so in test mode the trust anchor is
//! not the listener but the seam's own private folder: `digest` is the SHA-256 of the one release this run may install
//! (harness.py writes it from the bundle it packed), and the job refuses any other ([`Seam::trusts`]) before it
//! downloads anything. What a foreign listener can still do is what it could in phase 1: show a release on the card.
use super::{
    feed::Origin,
    http::Client,
    verify::{self, Hash},
    version::Version,
};
use std::{
    ffi::OsStr,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

pub(crate) const VAR: &str = "WIN11_WINPOINT_TEST";
const SEAM_FILE: &str = "seam.json";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Seam {
    Production,
    Test {
        origin: url::Url,
        running: Version,
        /// the SHA-256 of the one tarball this run may install (seam.json `digest`); None only in a unit test's seam
        digest: Option<Hash>,
        dir: Option<PathBuf>,
    },
}

impl Seam {
    /// The process's seam: production without the variable, else its checked test seam.
    pub(crate) fn process() -> Result<Self, String> {
        Self::check(std::env::var_os(VAR).as_deref(), unsafe { libc::geteuid() })
    }
    /// A unit test's seam (no environment, no directory, no pinned tarball: the fake's own digest is trusted).
    #[cfg(test)]
    pub(crate) fn for_test(origin: url::Url, running: Version) -> Self {
        Seam::Test {
            origin,
            running,
            digest: None,
            dir: None,
        }
    }
    /// A unit test's seam that pins the tarball, as a seam from the environment always does.
    #[cfg(test)]
    pub(crate) fn pinned(origin: url::Url, running: Version, digest: Hash) -> Self {
        Seam::Test {
            origin,
            running,
            digest: Some(digest),
            dir: None,
        }
    }
    /// The checks of the module doc on the variable's value.
    pub(crate) fn check(value: Option<&OsStr>, euid: u32) -> Result<Self, String> {
        let Some(value) = value else {
            return Ok(Seam::Production);
        };
        if euid == 0 {
            return Err(format!("{VAR}: euid 0"));
        }
        let dir = Path::new(value);
        if !dir.is_absolute() {
            return Err(format!("{VAR}: not an absolute path"));
        }
        let meta = std::fs::symlink_metadata(dir).map_err(|e| format!("{VAR}: {e}"))?;
        if !meta.is_dir() || meta.uid() != euid || meta.mode() & 0o777 != 0o700 {
            return Err(format!("{VAR}: not a private directory of ours (0700)"));
        }
        let ours = |name: &str| {
            std::fs::symlink_metadata(dir.join(name))
                .is_ok_and(|m| m.is_file() && m.uid() == euid && m.mode() & 0o022 == 0)
        };
        if !ours(crate::services::update::paths::MARKER) {
            return Err(format!("{VAR}: no fixture marker"));
        }
        if !ours(SEAM_FILE) {
            return Err(format!("{VAR}: no {SEAM_FILE}"));
        }
        // the directory is 0700 and ours, so nobody else can swap the file between these two reads
        let bytes =
            std::fs::read(dir.join(SEAM_FILE)).map_err(|e| format!("{VAR}: {SEAM_FILE}: {e}"))?;
        if bytes.len() > 4096 {
            return Err(format!("{VAR}: {SEAM_FILE} is too large"));
        }
        let (origin, running, digest) = parse_seam(&bytes)?;
        Ok(Seam::Test {
            origin,
            running,
            digest: Some(digest),
            dir: Some(dir.to_path_buf()),
        })
    }

    pub(crate) fn origin(&self) -> Origin {
        match self {
            Seam::Production => Origin::github(),
            Seam::Test { origin, .. } => Origin::test(origin),
        }
    }
    pub(crate) fn client(&self) -> Client {
        match self {
            Seam::Production => Client::production(),
            Seam::Test { origin, .. } => Client::for_test(origin.clone()),
        }
    }
    /// The version a test run pretends to be (it replaces this host's own and the installed one).
    pub(crate) fn running(&self) -> Option<&Version> {
        match self {
            Seam::Production => None,
            Seam::Test { running, .. } => Some(running),
        }
    }
    /// Whether a job may download and install the tarball whose GitHub digest is `digest` (job.rs, before the
    /// `.sha256`): in production yes (GitHub is the anchor, verify.rs); in test mode only the tarball seam.json pins
    /// (whoever listens on the loopback port cannot choose what runs); a unit test's seam pins none.
    pub(crate) fn trusts(&self, digest: &Hash) -> bool {
        match self {
            Seam::Production => true,
            Seam::Test {
                digest: Some(pinned),
                ..
            } => pinned == digest,
            Seam::Test {
                digest: None,
                dir: None,
                ..
            } => true,
            Seam::Test { .. } => false,
        }
    }
}

/// `{"origin":"http://127.0.0.1:<port>/","running":"<semver>","digest":"sha256:<64 lowercase hex>"}`, nothing else.
pub(crate) fn parse_seam(bytes: &[u8]) -> Result<(url::Url, Version, Hash), String> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| format!("{SEAM_FILE}: not JSON"))?;
    let object = value
        .as_object()
        .ok_or(format!("{SEAM_FILE}: not an object"))?;
    if object.len() != 3 {
        return Err(format!("{SEAM_FILE}: exactly origin, running and digest"));
    }
    let origin = object
        .get("origin")
        .and_then(|v| v.as_str())
        .ok_or(format!("{SEAM_FILE}: no origin"))?;
    let port = origin
        .strip_prefix("http://127.0.0.1:")
        .and_then(|rest| rest.strip_suffix('/'))
        .filter(|port| !port.starts_with('0') && port.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|port| port.parse::<u16>().ok())
        .filter(|port| *port >= 1024)
        .ok_or(format!(
            "{SEAM_FILE}: origin is not http://127.0.0.1:<1024-65535>/"
        ))?;
    let origin =
        url::Url::parse(&format!("http://127.0.0.1:{port}/")).map_err(|e| e.to_string())?;
    let running = object
        .get("running")
        .and_then(|v| v.as_str())
        .and_then(Version::parse)
        .ok_or(format!("{SEAM_FILE}: running is not a version"))?;
    let digest = object
        .get("digest")
        .and_then(|v| v.as_str())
        .and_then(verify::parse_digest)
        .ok_or(format!(
            "{SEAM_FILE}: digest is not sha256:<64 lowercase hex>"
        ))?;
    Ok((origin, running, digest))
}
