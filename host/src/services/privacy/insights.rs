//! Ubuntu's system reports (privacy.insights.*): GNOME Settings' Ubuntu Diagnostics page asks libinsights for the
//! consent of two sources ("Send a system report every month": the platform source `linux`; "Send a system report
//! when upgrading to a new Ubuntu version": `ubuntu_release_upgrader`) and compiles the report it shows. Amendment 38:
//! no Go runtime in the host, so the same through `/usr/bin/ubuntu-insights` (tools::system_tool):
//!
//! * `ubuntu-insights consent <source>` prints `<source>: true|false` on stdout; a source without a consent file is an
//!   error on stderr (exit 1), which GNOME shows as off (unknown consent);
//! * `ubuntu-insights consent <source> --state true|false` writes `~/.config/ubuntu-insights/<source>-consent.toml`;
//! * `ubuntu-insights collect --dry-run` prints the platform report (JSON) and writes nothing (no cache, no upload).
use serde_json::{Value, json};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The sources GNOME Settings shows, in its order.
pub(super) const SOURCES: &[&str] = &["linux", "ubuntu_release_upgrader"];
/// The source whose report "Diagnostische gegevens weergeven" shows (GNOME compiles the platform's).
pub(super) const REPORT_SOURCE: &str = "linux";
const CONSENT_SECONDS: u64 = 10;
/// More than a report collection takes on a slow machine; the dialog can cancel earlier.
const REPORT_SECONDS: u64 = 90;
/// A report is a few kB; anything far larger is not a report.
const REPORT_LIMIT: u64 = 4 * 1024 * 1024;

pub(super) fn source_ok(source: &str) -> bool {
    SOURCES.contains(&source)
}

/// The consent of each source in `stdout` (`linux: true`); a source the output does not name is unknown (None).
pub(super) fn parse_consent(stdout: &str) -> Vec<(&'static str, Option<bool>)> {
    SOURCES
        .iter()
        .map(|source| {
            let state = stdout.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                (name.trim() == *source)
                    .then(|| match value.trim() {
                        "true" => Some(true),
                        "false" => Some(false),
                        _ => None,
                    })
                    .flatten()
            });
            (*source, state)
        })
        .collect()
}

fn tool_str(tool: &Path) -> String {
    tool.to_string_lossy().into_owned()
}

/// `{sources: {linux: true|false|null, ubuntu_release_upgrader: ...}}` (one run per source: a missing consent file
/// of one source must not hide the other's).
pub(super) fn consent(tool: &Path) -> Value {
    let mut out = serde_json::Map::new();
    for source in SOURCES {
        let run = crate::services::system::common::run(
            &[&tool_str(tool), "consent", source],
            CONSENT_SECONDS,
            true,
        );
        let state = parse_consent(&run.out)
            .into_iter()
            .find(|(s, _)| s == source)
            .and_then(|(_, state)| state);
        out.insert((*source).into(), json!(state));
    }
    json!({ "sources": out })
}

/// Set one source's consent; the tool's error text when it fails.
pub(super) fn set_consent(tool: &Path, source: &str, on: bool) -> Result<(), String> {
    let state = if on { "true" } else { "false" };
    let run = crate::services::system::common::run(
        &[&tool_str(tool), "consent", source, "--state", state],
        CONSENT_SECONDS,
        true,
    );
    if run.ok {
        Ok(())
    } else {
        Err(run.err.trim().chars().take(300).collect())
    }
}

/// Why a report has no text.
#[derive(Debug, PartialEq)]
pub(super) enum ReportError {
    Cancelled,
    Failed(String),
}

/// `ubuntu-insights collect --dry-run`: the report's text, stopped when `cancel` is set (the dialog's "Annuleren",
/// GNOME's Cancel Report Generation) or after REPORT_SECONDS.
pub(super) fn report(tool: &Path, cancel: &AtomicBool) -> Result<String, ReportError> {
    let mut child = Command::new(tool)
        .args(["collect", "--dry-run"])
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| ReportError::Failed(e.to_string()))?;
    let (stdout, stderr) = (child.stdout.take(), child.stderr.take());
    let out = std::thread::spawn(move || {
        let mut data = Vec::new();
        if let Some(r) = stdout {
            let _ = r.take(REPORT_LIMIT).read_to_end(&mut data);
        }
        data
    });
    let err = std::thread::spawn(move || {
        let mut data = Vec::new();
        if let Some(r) = stderr {
            let _ = r.take(64 * 1024).read_to_end(&mut data);
        }
        String::from_utf8_lossy(&data).into_owned()
    });
    let end = Instant::now() + Duration::from_secs(REPORT_SECONDS);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Err(_) => break None,
            Ok(None) => {
                if cancel.load(Ordering::Acquire) || Instant::now() >= end {
                    let _ = child.kill();
                    let _ = child.wait();
                    // the readers end with the pipes (a grandchild may hold them): not waited for
                    drop((out, err));
                    return Err(if cancel.load(Ordering::Acquire) {
                        ReportError::Cancelled
                    } else {
                        ReportError::Failed("timeout".into())
                    });
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    };
    let data = out.join().unwrap_or_default();
    let err = err.join().unwrap_or_default();
    match status {
        Some(s) if s.success() => {
            let text = String::from_utf8_lossy(&data).trim().to_owned();
            pretty(&text).ok_or_else(|| ReportError::Failed("no report".into()))
        }
        _ => Err(ReportError::Failed(err.trim().chars().take(300).collect())),
    }
}

/// The report as GNOME shows it: the JSON, indented (None when the text is no JSON object).
pub(super) fn pretty(text: &str) -> Option<String> {
    let value: Value = serde_json::from_str(text).ok()?;
    value
        .is_object()
        .then(|| serde_json::to_string_pretty(&value).ok())
        .flatten()
}
