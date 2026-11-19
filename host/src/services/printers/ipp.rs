//! The scheduler's answers as Settings shows them (pure, tested on fixtures): a queue (GNOME's printer row: name,
//! model, location, state with its reasons, the ink and toner levels, its jobs, whether it can clean its heads) and a
//! job (GNOME's Jobs dialog: title, owner, state, held, waiting for authentication).
use super::cups::{Attrs, numbers, one, text, texts};
use serde_json::{Value, json};

/// The attributes `printers.list` asks for (CUPS-Get-Printers) and `printers.get` adds to its own.
pub(crate) const PRINTER_ATTRS: &[&str] = &[
    "printer-name",
    "printer-info",
    "printer-location",
    "printer-make-and-model",
    "printer-state",
    "printer-state-reasons",
    "printer-state-message",
    "printer-is-accepting-jobs",
    "printer-is-shared",
    "printer-is-temporary",
    "printer-type",
    "device-uri",
    "printer-uri-supported",
    "marker-names",
    "marker-levels",
    "marker-colors",
    "marker-types",
    "printer-commands",
    "queued-job-count",
    "auth-info-required",
];

/// The attributes of a job in GNOME's Jobs dialog.
pub(crate) const JOB_ATTRS: &[&str] = &[
    "job-id",
    "job-name",
    "job-state",
    "job-state-reasons",
    "job-originating-user-name",
    "job-printer-uri",
    "job-priority",
    "job-k-octets",
    "time-at-creation",
    "time-at-processing",
    "job-hold-until",
    "job-media-sheets-completed",
];

/// CUPS's printer-type bits Settings reads.
const TYPE_CLASS: i64 = 0x0001;
const TYPE_REMOTE: i64 = 0x0002;
const TYPE_COLOR: i64 = 0x0008;
const TYPE_FAX: i64 = 0x40000;
const TYPE_SCANNER: i64 = 0x2000000;
const TYPE_MFP: i64 = 0x4000000;

/// printer-state as a word.
pub(crate) fn state_name(state: i64) -> &'static str {
    match state {
        3 => "idle",
        4 => "processing",
        5 => "stopped",
        _ => "unknown",
    }
}

/// A printer-state-reasons keyword without its severity suffix, and the severity ("error", "warning", "report").
pub(crate) fn reason(keyword: &str) -> (String, &'static str) {
    for (suffix, severity) in [
        ("-error", "error"),
        ("-warning", "warning"),
        ("-report", "report"),
    ] {
        if let Some(base) = keyword.strip_suffix(suffix) {
            return (base.to_owned(), severity);
        }
    }
    // without a suffix a reason is an error (RFC 8011 5.4.12), except the plain states CUPS reports
    let severity = if matches!(
        keyword,
        "paused" | "moving-to-paused" | "shutdown" | "deactivated"
    ) {
        "report"
    } else {
        "error"
    };
    (keyword.to_owned(), severity)
}

/// The reasons GNOME names (pp-printer-entry.c): the rest ("none", "cups-*" bookkeeping) is not shown.
pub(crate) fn shown_reason(base: &str) -> bool {
    !matches!(
        base,
        "none" | "cups-waiting-for-job-completed" | "cups-ipp-conformance-failure" | "other"
    ) && !base.starts_with("com.")
}

/// The ink, toner and other supplies (marker-*): one entry per marker; a level below 0 is unknown.
pub(crate) fn markers(attrs: &Attrs) -> Vec<Value> {
    let names = texts(attrs, "marker-names");
    let levels = numbers(attrs, "marker-levels");
    let colors = texts(attrs, "marker-colors");
    let kinds = texts(attrs, "marker-types");
    names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let level = levels.get(i).copied().filter(|l| (0..=100).contains(l));
            // "#RRGGBB" or several colours of one cartridge ("#00FFFF#FF00FF#FFFF00")
            let color: Vec<String> = colors
                .get(i)
                .map(|c| {
                    c.split('#')
                        .filter(|p| p.len() == 6 && p.chars().all(|ch| ch.is_ascii_hexdigit()))
                        .map(|p| format!("#{}", p.to_ascii_lowercase()))
                        .collect()
                })
                .unwrap_or_default();
            json!({
                "name": name,
                "level": level,
                "colors": color,
                "type": kinds.get(i).cloned().unwrap_or_default(),
            })
        })
        .collect()
}

/// Whether the queue can clean its print heads: printer-commands has "Clean" (CUPS copies the PPD's cupsCommands
/// there), as GNOME's maintenance command checks.
pub(crate) fn can_clean(attrs: &Attrs) -> bool {
    texts(attrs, "printer-commands")
        .iter()
        .any(|c| c.eq_ignore_ascii_case("clean"))
}

/// One queue for `printers.list` / `printers.get`.
pub(crate) fn printer(attrs: &Attrs, default: Option<&str>) -> Value {
    let name = text(attrs, "printer-name");
    let kind = one(attrs, "printer-type").as_i64().unwrap_or(0);
    let state = one(attrs, "printer-state").as_i64().unwrap_or(0);
    let mut reasons: Vec<Value> = Vec::new();
    for keyword in texts(attrs, "printer-state-reasons") {
        let (base, severity) = reason(&keyword);
        if shown_reason(&base) && !reasons.iter().any(|r| r["reason"] == base.as_str()) {
            reasons.push(json!({"reason": base, "severity": severity}));
        }
    }
    let info = text(attrs, "printer-info");
    json!({
        "name": name,
        "info": info,
        "title": if info.trim().is_empty() { name.clone() } else { info.clone() },
        "location": text(attrs, "printer-location"),
        "makeModel": text(attrs, "printer-make-and-model"),
        "state": state_name(state),
        "reasons": reasons,
        "message": text(attrs, "printer-state-message"),
        "accepting": one(attrs, "printer-is-accepting-jobs").as_bool().unwrap_or(true),
        "shared": one(attrs, "printer-is-shared").as_bool().unwrap_or(false),
        "isClass": kind & TYPE_CLASS != 0,
        "remote": kind & TYPE_REMOTE != 0,
        "color": kind & TYPE_COLOR != 0,
        "fax": kind & TYPE_FAX != 0,
        "scanner": kind & (TYPE_SCANNER | TYPE_MFP) != 0,
        "deviceUri": text(attrs, "device-uri"),
        "markers": markers(attrs),
        "commands": texts(attrs, "printer-commands").into_iter().filter(|c| c != "none").collect::<Vec<_>>(),
        "canClean": can_clean(attrs),
        "jobs": one(attrs, "queued-job-count").as_i64().unwrap_or(0),
        "authInfoRequired": texts(attrs, "auth-info-required").into_iter().filter(|a| a != "none").collect::<Vec<_>>(),
        "isDefault": default.is_some_and(|d| d == name),
    })
}

/// A queue Settings lists: not one of CUPS's temporary queues for a printer that was only printed to once
/// (printer-is-temporary), which g-c-c does not list as installed either.
pub(crate) fn listed(attrs: &Attrs) -> bool {
    !text(attrs, "printer-name").is_empty() && one(attrs, "printer-is-temporary") != &json!(true)
}

/// job-state as a word.
pub(crate) fn job_state(state: i64) -> &'static str {
    match state {
        3 => "pending",
        4 => "held",
        5 => "processing",
        6 => "stopped",
        7 => "canceled",
        8 => "aborted",
        9 => "completed",
        _ => "unknown",
    }
}

/// The queue name at the end of a printer URI (job-printer-uri).
pub(crate) fn queue_of(uri: &str) -> String {
    let tail = uri
        .rsplit_once("/printers/")
        .or_else(|| uri.rsplit_once("/classes/"))
        .map(|(_, t)| t)
        .unwrap_or("");
    percent_decode(tail.split('?').next().unwrap_or(""))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(b) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// One job for `printers.jobs`.
pub(crate) fn job(attrs: &Attrs) -> Value {
    let state = one(attrs, "job-state").as_i64().unwrap_or(0);
    let reasons = texts(attrs, "job-state-reasons");
    let hold = text(attrs, "job-hold-until");
    json!({
        "id": one(attrs, "job-id").as_i64().unwrap_or(0),
        "name": text(attrs, "job-name"),
        "state": job_state(state),
        "user": text(attrs, "job-originating-user-name"),
        "printer": queue_of(&text(attrs, "job-printer-uri")),
        "priority": one(attrs, "job-priority").as_i64().unwrap_or(50),
        "size": one(attrs, "job-k-octets").as_i64().unwrap_or(0),
        "created": one(attrs, "time-at-creation").as_i64().unwrap_or(0),
        "processing": one(attrs, "time-at-processing").as_i64().filter(|t| *t > 0),
        "sheets": one(attrs, "job-media-sheets-completed").as_i64().unwrap_or(0),
        "held": state == 4,
        "holdUntil": hold,
        "authRequired": reasons.iter().any(|r| r == "cups-held-for-authentication"),
        "reasons": reasons.into_iter().filter(|r| r != "none").collect::<Vec<_>>(),
        "done": state >= 7,
    })
}

/// A printer-device-id ("MFG:HP;MDL:LaserJet M404;CMD:PCL,PDF;") as (manufacturer, model).
pub(crate) fn device_id(id: &str) -> (String, String) {
    let mut make = String::new();
    let mut model = String::new();
    for part in id.split(';') {
        let Some((key, value)) = part.split_once(':') else {
            continue;
        };
        let value = value.trim().to_owned();
        match key.trim().to_ascii_uppercase().as_str() {
            "MFG" | "MANUFACTURER" if make.is_empty() => make = value,
            "MDL" | "MODEL" if model.is_empty() => model = value,
            _ => {}
        }
    }
    (make, model)
}

/// Whether a printer answers as an IPP Everywhere (or AirPrint-like driverless) printer: IPP 2.0 and a raster or PDF
/// format cupsd's `everywhere` model can drive (pwg-raster, urf, pdf), the test of amendment 41.
pub(crate) fn driverless(attrs: &Attrs) -> bool {
    let formats = texts(attrs, "document-format-supported");
    let ipp2 = texts(attrs, "ipp-versions-supported")
        .iter()
        .any(|v| v.as_str() >= "2.0");
    ipp2 && formats.iter().any(|f| {
        matches!(
            f.as_str(),
            "image/pwg-raster" | "image/urf" | "application/pdf"
        )
    })
}
