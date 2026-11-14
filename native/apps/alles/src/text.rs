//! Alles's own texts (vendor/ui/js/apps/alles/i18n.js): the plural forms, the filter and column names and the
//! Windows-style type names. Every other text is written in place as `t(nl, en)` with i18n.js's wording.
use kit::format;
use kit::t;

use crate::model::Entry;

/// The filters of the "Soort" drop-down, in i18n.js FILTERS order.
pub const FILTERS: [&str; 8] = ["all", "audio", "compressed", "document", "executable", "folder", "picture", "video"];

/// The sort keys the backend knows (the four columns plus "type").
pub const SORTS: [&str; 5] = ["name", "path", "size", "modified", "type"];

/// i18n.js f_<filter>.
pub fn filter_label(f: &str) -> &'static str {
    match f {
        "audio" => "Audio",
        "compressed" => t("Archief (zip)", "Archive (zip)"),
        "document" => "Document",
        "executable" => t("Programma", "Program"),
        "folder" => t("Map", "Folder"),
        "picture" => t("Afbeelding", "Picture"),
        "video" => "Video",
        _ => t("Alles", "Everything"),
    }
}

/// i18n.js col_<key>.
pub fn column_label(c: &str) -> &'static str {
    match c {
        "path" => t("Pad", "Path"),
        "size" => t("Grootte", "Size"),
        "modified" => t("Gewijzigd op", "Date modified"),
        "type" => "Type",
        _ => t("Naam", "Name"),
    }
}

/// A number as the UI writes it (format.js number).
pub fn num(n: f64) -> String {
    format::number(n, 0)
}

/// i18n.js objects {zero, one, other}.
pub fn objects(n: u64) -> String {
    match n {
        0 => t("Geen objecten", "No objects").to_owned(),
        1 => t("1 object", "1 object").to_owned(),
        _ => format!("{} {}", num(n as f64), t("objecten", "objects")),
    }
}

/// i18n.js welcomeBody {one, other}.
pub fn welcome_body(n: u64) -> String {
    if n == 1 {
        t(
            "Typ een deel van een naam. Alles doorzoekt 1 bestand of map terwijl je typt.",
            "Type part of a name. Alles searches 1 file or folder as you type.",
        )
        .to_owned()
    } else {
        let c = num(n as f64);
        if kit::i18n::en() {
            format!("Type part of a name. Alles searches {c} files and folders as you type.")
        } else {
            format!("Typ een deel van een naam. Alles doorzoekt {c} bestanden en mappen terwijl je typt.")
        }
    }
}

/// i18n.js firstFound {one, other}.
pub fn first_found(n: u64) -> String {
    if n == 1 {
        t("1 item gevonden", "1 item found").to_owned()
    } else {
        format!("{} {}", num(n as f64), t("items gevonden", "items found"))
    }
}

/// i18n.js pv_multi {one, other}.
pub fn items_selected(n: u64) -> String {
    if n == 1 {
        t("1 item geselecteerd", "1 item selected").to_owned()
    } else {
        format!("{} {}", num(n as f64), t("items geselecteerd", "items selected"))
    }
}

/// i18n.js trashTitle {one, other}.
pub fn trash_title(n: usize) -> String {
    if n == 1 {
        t("Naar de Prullenbak verplaatsen?", "Move to the Recycle Bin?").to_owned()
    } else if kit::i18n::en() {
        format!("Move {} items to the Recycle Bin?", num(n as f64))
    } else {
        format!("{} items naar de Prullenbak verplaatsen?", num(n as f64))
    }
}

/// i18n.js trashed {one, other}.
pub fn trashed(n: usize) -> String {
    if n == 1 {
        t("Verplaatst naar de Prullenbak", "Moved to the Recycle Bin").to_owned()
    } else if kit::i18n::en() {
        format!("{} items moved to the Recycle Bin", num(n as f64))
    } else {
        format!("{} items verplaatst naar de Prullenbak", num(n as f64))
    }
}

/// i18n.js trashFailedBody {one, other} (D-G1a/b: Dutch keeps "item(s)" for the other forms).
pub fn trash_failed_body(n: usize) -> String {
    match (n == 1, kit::i18n::en()) {
        (true, false) => format!("{n} item kon niet worden verplaatst. Misschien heb je er geen rechten voor."),
        (true, true) => format!("{n} item couldn’t be moved. You might not have permission."),
        (false, false) => format!("{n} item(s) konden niet worden verplaatst. Misschien heb je er geen rechten voor."),
        (false, true) => format!("{n} items couldn’t be moved. You might not have permission."),
    }
}

/// i18n.js trashTooMany {one, other} (n is the limit).
pub fn trash_too_many(n: usize) -> String {
    match (n == 1, kit::i18n::en()) {
        (true, false) => format!("Kies maximaal {n} item om tegelijk naar de Prullenbak te verplaatsen."),
        (true, true) => format!("Choose at most {n} item to move to the Recycle Bin at a time."),
        (false, false) => format!("Kies maximaal {n} items om tegelijk naar de Prullenbak te verplaatsen."),
        (false, true) => format!("Choose at most {n} items to move to the Recycle Bin at a time."),
    }
}

/// i18n.js copied / namesCopied {one, other}.
pub fn copied(n: usize, names: bool) -> String {
    let c = num(n as f64);
    match (names, n == 1) {
        (false, true) => t("Pad gekopieerd", "Path copied").to_owned(),
        (false, false) => format!("{c} {}", t("paden gekopieerd", "paths copied")),
        (true, true) => t("Naam gekopieerd", "Name copied").to_owned(),
        (true, false) => format!("{c} {}", t("namen gekopieerd", "names copied")),
    }
}

/// i18n.js copyCapped {one, other}.
pub fn copy_capped(n: usize) -> String {
    if n == 1 {
        t("Alleen het eerste pad is gekopieerd.", "Only the first path was copied.").to_owned()
    } else if kit::i18n::en() {
        format!("Only the first {} paths were copied.", num(n as f64))
    } else {
        format!("Alleen de eerste {} paden zijn gekopieerd.", num(n as f64))
    }
}

/// i18n.js pr_containsVal ("12 bestanden, 3 mappen"; English counts 1 file / 1 folder).
pub fn contains_value(files: f64, folders: f64) -> String {
    let (f, d) = (num(files), num(folders));
    if kit::i18n::en() {
        format!("{f} {}, {d} {}", if f == "1" { "file" } else { "files" }, if d == "1" { "folder" } else { "folders" })
    } else {
        format!("{f} bestanden, {d} mappen")
    }
}

/// i18n.js tookMs: "{ms} ms" with one decimal below 10.
pub fn took_ms(ms: f64) -> String {
    format!("{} ms", format::number(ms, if ms < 10.0 { 1 } else { 0 }))
}

/// i18n.js typeName: a friendly type name, Windows Explorer style ("PDF-document", "JPEG-afbeelding").
pub fn type_name(e: &Entry) -> String {
    if e.is_dir {
        return t("Bestandsmap", "File folder").to_owned();
    }
    let ext = e.ext.to_lowercase();
    let upper = if ext == "jpg" { "JPEG".to_owned() } else { ext.to_uppercase() };
    let en = kit::i18n::en();
    let with = |nl: &str, en_text: &str| -> String {
        if en { en_text.replace("{ext}", &upper) } else { nl.replace("{ext}", &upper) }
    };
    let known = match ext.as_str() {
        "pdf" => Some(with("PDF-document", "PDF Document")),
        "doc" | "docx" => Some(with("Word-document", "Word Document")),
        "xls" | "xlsx" => Some(with("Excel-werkblad", "Excel Worksheet")),
        "ppt" | "pptx" => Some(with("PowerPoint-presentatie", "PowerPoint Presentation")),
        "odt" => Some(with("OpenDocument-tekst", "OpenDocument Text")),
        "ods" => Some(with("OpenDocument-werkblad", "OpenDocument Spreadsheet")),
        "odp" => Some(with("OpenDocument-presentatie", "OpenDocument Presentation")),
        "txt" | "log" => Some(with("Tekstdocument", "Text Document")),
        "md" => Some(with("Markdown-document", "Markdown Document")),
        "html" | "htm" => Some(with("Webpagina", "Web Page")),
        "iso" | "img" => Some(with("Schijfkopie", "Disc Image")),
        "deb" | "rpm" | "msi" => Some(with("Installatiepakket", "Installer Package")),
        "appimage" => Some(with("Programma (AppImage)", "Application (AppImage)")),
        "sh" | "bash" | "ps1" | "bat" => Some(with("Script", "Script")),
        "desktop" => Some(with("Snelkoppeling", "Shortcut")),
        "exe" | "run" => Some(with("Programma", "Application")),
        "qcow2" | "vdi" | "vmdk" | "vhdx" => Some(with("Virtuele schijf", "Virtual Disk")),
        "srt" | "vtt" => Some(with("Ondertiteling", "Subtitles")),
        "js" | "ts" | "py" | "c" | "cpp" | "h" | "rs" | "go" | "java" | "css" => {
            Some(with("Broncode ({ext})", "Source Code ({ext})"))
        }
        "json" | "xml" | "csv" | "yaml" | "yml" | "toml" | "ini" => Some(with("Gegevens ({ext})", "Data ({ext})")),
        "zip" | "7z" | "rar" | "gz" | "xz" | "tar" | "zst" | "bz2" => Some(with("{ext}-archief", "{ext} Archive")),
        _ => None,
    };
    if let Some(k) = known {
        return k;
    }
    let mime = e.mime.as_str();
    if mime.starts_with("image/") {
        // D-G1b keeps G's Dutch "Afbeelding-afbeelding" for an image without an extension
        return if upper.is_empty() { with("Afbeelding-afbeelding", "Image") } else { with("{ext}-afbeelding", "{ext} Image") };
    }
    if mime.starts_with("audio/") {
        return with("{ext}-audiobestand", "{ext} Audio");
    }
    if mime.starts_with("video/") {
        return with("{ext}-video", "{ext} Video");
    }
    if ext.is_empty() {
        return if mime == "application/x-executable" { with("Programma", "Application") } else { with("Bestand", "File") };
    }
    with("{ext}-bestand", "{ext} File")
}
