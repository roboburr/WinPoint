//! Native XBEL history and privacy clearing, preserving the original Apps contract.
use crate::{
    events::Events,
    files::{self, ApiError, ApiResult},
};
use gio::prelude::*;
use gtk::{gio, glib};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

#[path = "recent_datetime.rs"]
mod datetime;

const MIME_NS: &str = "http://www.freedesktop.org/standards/shared-mime-info";
const BOOKMARK_NS: &str = "http://www.freedesktop.org/standards/desktop-bookmarks";
const MAX_XML: u64 = 64 * 1024 * 1024;
const EMPTY_XBEL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xbel version=\"1.0\"\n      xmlns:bookmark=\"http://www.freedesktop.org/standards/desktop-bookmarks\"\n      xmlns:mime=\"http://www.freedesktop.org/standards/shared-mime-info\"\n>\n</xbel>\n";

#[derive(Clone, Debug, PartialEq)]
struct Bookmark {
    path: String,
    uri: String,
    mime: String,
    modified: i64,
    app: String,
}
#[derive(Default)]
struct Cache {
    key: Option<(PathBuf, i64, i64, u64)>,
    items: Vec<Bookmark>,
}
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
static CLEAR: Mutex<()> = Mutex::new(());

pub(super) fn call(
    method: &str,
    args: &Value,
    settings: &Value,
    events: &Arc<Events>,
) -> Option<ApiResult> {
    if !matches!(method, "apps.recent" | "apps.clearRecent") {
        return None;
    }
    let data = glib::user_data_dir();
    let path = std::env::var_os("WIN11SHELL_RECENT_FILE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| data.join("recently-used.xbel"));
    Some(call_at(method, args, settings, events, &path, &data))
}
fn call_at(
    method: &str,
    args: &Value,
    settings: &Value,
    events: &Arc<Events>,
    path: &Path,
    data: &Path,
) -> ApiResult {
    if method == "apps.recent" {
        if settings
            .get("privacy")
            .and_then(|value| value.get("recentFiles"))
            .is_some_and(|value| !truthy(value))
        {
            return Ok(json!([]));
        }
        return Ok(recent(path, data, limit(&args["limit"])));
    }
    let result = clear(path, crate::i18n::Lang::of_settings(settings))?;
    events.emit("apps.changed", json!({}));
    Ok(result)
}
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    }
}
fn limit(value: &Value) -> usize {
    match value {
        Value::Bool(_) => 1,
        Value::Number(value) => value
            .as_f64()
            .map(|v| v.trunc().clamp(1.0, 200.0) as usize)
            .unwrap_or(12),
        Value::String(value) => {
            let value = value
                .trim_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c));
            let (negative, value) = if let Some(value) = value.strip_prefix('-') {
                (true, value)
            } else {
                (false, value.strip_prefix('+').unwrap_or(value))
            };
            let mut total = 0_usize;
            let mut digits = 0;
            let mut underscore = true;
            for c in value.chars() {
                if c == '_' && !underscore {
                    underscore = true;
                    continue;
                }
                let digit = unsafe { glib::ffi::g_unichar_digit_value(c as u32) };
                if digit < 0 {
                    return 12;
                }
                total = (total * 10 + digit as usize).min(1000);
                digits += 1;
                if digits > 4300 {
                    return 12;
                }
                underscore = false;
            }
            if digits == 0 || underscore {
                12
            } else if negative {
                1
            } else {
                total.clamp(1, 200)
            }
        }
        _ => 12,
    }
}

fn read_history(path: &Path) -> io::Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::from_raw_os_error(libc::EINVAL));
    }
    let mut bytes = Vec::new();
    file.take(MAX_XML + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_XML {
        return Err(io::Error::from_raw_os_error(libc::EFBIG));
    }
    Ok(bytes)
}
fn xml_text(bytes: &[u8]) -> Option<String> {
    let encoding = if bytes.starts_with(&[0xff, 0xfe, 0, 0]) || bytes.starts_with(&[b'<', 0, 0, 0])
    {
        "UTF-32LE".to_owned()
    } else if bytes.starts_with(&[0, 0, 0xfe, 0xff]) || bytes.starts_with(&[0, 0, 0, b'<']) {
        "UTF-32BE".to_owned()
    } else if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[b'<', 0, b'?', 0]) {
        "UTF-16LE".to_owned()
    } else if bytes.starts_with(&[0xfe, 0xff]) || bytes.starts_with(&[0, b'<', 0, b'?']) {
        "UTF-16BE".to_owned()
    } else {
        let prefix = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
        let mut encoding = "UTF-8".to_owned();
        if prefix.starts_with(b"<?xml") {
            let end = prefix.windows(2).position(|value| value == b"?>")?;
            let declaration = std::str::from_utf8(&prefix[..end]).ok()?;
            if let Some((_, tail)) = declaration.split_once("encoding") {
                let tail = tail.trim_start().strip_prefix('=')?.trim_start();
                let quote = tail.chars().next()?;
                if quote != '\'' && quote != '"' {
                    return None;
                }
                encoding = tail[1..].split(quote).next()?.to_owned();
            }
        }
        encoding
    };
    let text = if encoding.eq_ignore_ascii_case("UTF-8") || encoding.eq_ignore_ascii_case("UTF8") {
        String::from_utf8(bytes.to_vec()).ok()?
    } else {
        let (decoded, consumed) = glib::convert(bytes, "UTF-8", encoding.as_str()).ok()?;
        if consumed != bytes.len() {
            return None;
        }
        String::from_utf8(decoded.to_vec()).ok()?
    };
    Some(text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned())
}
fn uri_path(uri: &str) -> Option<String> {
    if !uri.starts_with("file://") {
        return None;
    }
    // Python urlsplit removes literal ASCII tab/newline/CR, but preserves path spelling.
    let clean: String = uri
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    let tail = clean.strip_prefix("file://")?;
    let end = tail.find(['/', '?', '#']).unwrap_or(tail.len());
    if !matches!(&tail[..end], "" | "localhost") {
        return None;
    }
    let path = &tail[end..];
    let path = &path[..path.find(['?', '#']).unwrap_or(path.len())];
    if path.is_empty() {
        return None;
    }
    let mut decoded = Vec::with_capacity(path.len());
    let bytes = path.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(a), Some(b)) = (
                (bytes[index + 1] as char).to_digit(16),
                (bytes[index + 2] as char).to_digit(16),
            ) {
                decoded.push((a * 16 + b) as u8);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    Some(String::from_utf8_lossy(&decoded).into_owned())
}
fn parse_xbel(bytes: &[u8]) -> Vec<Bookmark> {
    let Some(text) = xml_text(bytes) else {
        return Vec::new();
    };
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        nodes_limit: 1_000_000,
    };
    let Ok(tree) = roxmltree::Document::parse_with_options(&text, options) else {
        return Vec::new();
    };
    if !valid_internal_entities(&text[..tree.root_element().range().start]) {
        return Vec::new();
    }
    let mut items: Vec<Bookmark> = Vec::new();
    let mut positions: HashMap<String, usize> = HashMap::new();
    for bm in tree.descendants().filter(|node| {
        node.is_element()
            && node.tag_name().name() == "bookmark"
            && node.tag_name().namespace().is_none()
    }) {
        let uri = bm.attribute("href").unwrap_or("");
        let Some(path) = uri_path(uri) else {
            continue;
        };
        let mut modified = ["modified", "visited", "added"]
            .iter()
            .map(|key| datetime::iso_ms(bm.attribute(*key)))
            .max()
            .unwrap_or(0);
        let mut mime = "";
        let mut app = "";
        for child in bm.descendants().filter(|node| node.is_element()) {
            let tag = child.tag_name();
            if tag.namespace() == Some(MIME_NS) && tag.name() == "mime-type" {
                if let Some(value) = child.attribute("type").filter(|value| !value.is_empty()) {
                    mime = value;
                }
            } else if tag.namespace() == Some(BOOKMARK_NS) && tag.name() == "application" {
                modified = modified.max(datetime::iso_ms(child.attribute("modified")));
                if let Some(value) = child.attribute("name").filter(|value| !value.is_empty()) {
                    app = value;
                }
            }
        }
        let item = Bookmark {
            path: path.clone(),
            uri: uri.into(),
            mime: mime.into(),
            modified,
            app: app.into(),
        };
        if let Some(index) = positions.get(&path).copied() {
            if item.modified > items[index].modified {
                items[index] = item;
            }
        } else {
            positions.insert(path, items.len());
            items.push(item);
        }
    }
    items.sort_by_key(|item| std::cmp::Reverse(item.modified));
    items
}
fn valid_internal_entities(prolog: &str) -> bool {
    // Expat rejects literal parameter-entity markers inside an internal ENTITY
    // declaration's quoted value. roxmltree leaves that DTD check to its caller.
    // System/public identifiers may contain percent-encoded URIs and are left alone.
    let bytes = prolog.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"<!--") {
            let Some(end) = prolog[index + 4..].find("-->") else {
                return false;
            };
            index += 4 + end + 3;
            continue;
        }
        if bytes[index..].starts_with(b"<!ENTITY") {
            index += 8;
            while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
                index += 1;
            }
            if bytes.get(index) == Some(&b'%') {
                index += 1;
                while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
                    index += 1;
                }
            }
            while bytes
                .get(index)
                .is_some_and(|byte| !byte.is_ascii_whitespace())
            {
                index += 1;
            }
            while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
                index += 1;
            }
            if let Some(quote @ (b'\'' | b'"')) = bytes.get(index).copied() {
                index += 1;
                while let Some(byte) = bytes.get(index).copied() {
                    if byte == quote {
                        break;
                    }
                    if byte == b'%' {
                        return false;
                    }
                    index += 1;
                }
                index += 1;
            }
            continue;
        }
        index += 1;
    }
    true
}
fn entries(path: &Path) -> Vec<Bookmark> {
    let Ok(stat) = fs::metadata(path) else {
        return Vec::new();
    };
    let key = (path.to_owned(), stat.mtime(), stat.mtime_nsec(), stat.len());
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(Cache::default()))
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if cache.key.as_ref() != Some(&key) {
        cache.items = read_history(path)
            .map(|bytes| parse_xbel(&bytes))
            .unwrap_or_default();
        cache.key = Some(key);
    }
    cache.items.clone()
}
fn mime_icon(mime: &str) -> String {
    let mime = mime.trim().to_lowercase();
    if !mime.contains('/') || mime.chars().count() > 200 {
        return "text-x-generic".into();
    }
    let mut names = Vec::new();
    if let Ok(icon) = gio::content_type_get_icon(&mime).downcast::<gio::ThemedIcon>() {
        names.extend(icon.names().iter().map(|name| name.to_string()));
    }
    if let Some(name) = gio::content_type_get_generic_icon_name(&mime) {
        names.push(name.to_string());
    }
    let major = mime.split('/').next().unwrap();
    names.extend([mime.replace('/', "-"), format!("{major}-x-generic")]);
    if mime == "inode/directory" {
        names.insert(0, "folder".into());
    }
    names.push(
        if major == "text" {
            "text-x-generic"
        } else {
            "application-x-generic"
        }
        .into(),
    );
    names.push("unknown".into());
    names.sort_by_key(|name| name.ends_with("-symbolic"));
    let mut seen = HashSet::new();
    names.retain(|name| seen.insert(name.clone()));
    files::first_icon(
        &names.iter().map(String::as_str).collect::<Vec<_>>(),
        "text-x-generic",
    )
}
fn recent(path: &Path, data: &Path, limit: usize) -> Value {
    let trash = format!("{}/", data.join("Trash").display());
    let mut out = Vec::new();
    for (checked, item) in entries(path).into_iter().enumerate() {
        if out.len() >= limit || checked > 2000 {
            break;
        }
        if item.path.starts_with(&trash) {
            continue;
        }
        let Ok(stat) = fs::metadata(&item.path) else {
            continue;
        };
        let is_dir = stat.is_dir();
        let mime = if is_dir {
            "inode/directory".into()
        } else if item.mime.is_empty() {
            files::filename_mime(&item.path)
        } else {
            item.mime
        };
        let icon = if is_dir {
            "folder".into()
        } else {
            mime_icon(&mime)
        };
        let name = item
            .path
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .unwrap_or(&item.path);
        let modified = if item.modified != 0 {
            item.modified
        } else {
            ((stat.mtime() as f64 + stat.mtime_nsec() as f64 / 1e9) * 1000.0) as i64
        };
        out.push(json!({"path":item.path,"uri":item.uri,"name":name,"mime":mime,"icon":icon,"modified":modified,"isDir":is_dir,"app":item.app}));
    }
    Value::Array(out)
}
fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)?;
    let temporary = parent.join(format!(".recent-{}.tmp", crate::events::nonce()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
fn clear(path: &Path, lang: crate::i18n::Lang) -> ApiResult {
    let _lock = CLEAR.lock().unwrap_or_else(|error| error.into_inner());
    let failed = |_| ApiError::tr("failed", lang, "apps.historyNotCleared", crate::i18n::NO_PARAMS);
    let bytes = match read_history(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(failed(error)),
    };
    let count = bytes
        .as_deref()
        .map(parse_xbel)
        .map_or(0, |items| items.len());
    let backup = if let Some(bytes) = bytes {
        let mut name = path
            .file_name()
            .ok_or_else(|| failed(io::Error::from_raw_os_error(libc::EINVAL)))?
            .to_os_string();
        name.push(".bak");
        let backup = path.with_file_name(name);
        atomic_write(&backup, &bytes).map_err(failed)?;
        Some(backup)
    } else {
        None
    };
    atomic_write(path, EMPTY_XBEL.as_bytes()).map_err(failed)?;
    if let Some(cache) = CACHE.get() {
        *cache.lock().unwrap_or_else(|error| error.into_inner()) = Cache::default();
    }
    Ok(json!({"cleared":count,"backup":backup}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{os::unix::fs::symlink, time::Instant};
    struct Fixture {
        root: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("win11-recent-{}", crate::events::nonce()));
            fs::create_dir(&root).unwrap();
            Self { root }
        }
        fn history(&self) -> PathBuf {
            self.root.join("recently-used.xbel")
        }
        fn write(&self, text: &str) {
            fs::write(self.history(), text).unwrap();
        }
        fn uri(&self, name: &str) -> String {
            format!("file://{}", self.root.join(name).display())
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700));
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    #[test]
    fn namespaces_duplicates_stable_sort_and_last_application_follow_reference() {
        let xml = format!(
            r#"<xbel xmlns:b="{BOOKMARK_NS}" xmlns:m="{MIME_NS}" xmlns:other="urn:other">
          <bookmark href="file:///tmp/a" modified="2026-11-01T12:00:00Z"><info><m:mime-type type="image/png"/><b:application name="newer" modified="2026-11-01T14:00:00Z"/><b:application name="last-older" modified="2026-11-01T11:00:00Z"/></info></bookmark>
          <bookmark href="file:///tmp/b" modified="2026-11-01T14:00:00Z"/>
          <bookmark href="file://localhost/tmp/a" modified="2026-11-01T14:00:00Z"/>
          <other:bookmark href="file:///tmp/ignored" modified="9999-12-31T23:59:59Z"/>
          <bookmark href="https://example.invalid/ignored"/>
        </xbel>"#
        );
        let rows = parse_xbel(xml.as_bytes());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].path, "/tmp/a");
        assert_eq!(rows[0].uri, "file:///tmp/a");
        assert_eq!(rows[0].mime, "image/png");
        assert_eq!(rows[0].app, "last-older");
        assert_eq!(rows[0].modified, 1793541600000);
        assert_eq!(rows[1].path, "/tmp/b");
        let newer = format!("{xml}<bad>");
        assert!(parse_xbel(newer.as_bytes()).is_empty());
    }
    #[test]
    fn cdata_internal_entities_unicode_and_supported_xml_encodings() {
        let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><!DOCTYPE xbel [<!ENTITY place '/tmp/één'>]><xbel><bookmark href=\"file://&place;\"><title><![CDATA[<not-a-tag>&entity;]]></title></bookmark></xbel>";
        assert_eq!(parse_xbel(xml.as_bytes())[0].path, "/tmp/één");
        let utf16: Vec<u8> = "<?xml version=\"1.0\" encoding=\"UTF-16\"?><xbel><bookmark href=\"file:///tmp/één\"/></xbel>".encode_utf16().flat_map(u16::to_le_bytes).collect();
        let mut bom = vec![0xff, 0xfe];
        bom.extend(utf16);
        assert_eq!(parse_xbel(&bom)[0].path, "/tmp/één");
        let latin = b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><xbel><bookmark href=\"file:///tmp/caf\xe9\"/></xbel>";
        assert_eq!(parse_xbel(latin)[0].path, "/tmp/café");
    }
    #[test]
    fn hostile_xml_never_resolves_external_entities_and_malformed_input_is_empty() {
        let f = Fixture::new();
        let secret = f.root.join("secret");
        fs::write(&secret, b"fixture-only secret").unwrap();
        for xml in [
            format!("<!DOCTYPE xbel [<!ENTITY external SYSTEM 'file://{}'>]><xbel><bookmark href='file:///tmp/&external;'/></xbel>", secret.display()),
            "<!DOCTYPE xbel [<!ENTITY external SYSTEM 'http://127.0.0.1:9/no-network'>]><xbel><bookmark href='file:///tmp/&external;'/></xbel>".into(),
            "<!DOCTYPE xbel [<!ENTITY a '&b;'><!ENTITY b '&a;'>]><xbel><bookmark href='file:///tmp/&a;'/></xbel>".into(),
            "<!DOCTYPE xbel [<!ENTITY place 'file:///tmp/a%20b'>]><xbel><bookmark href='&place;'/></xbel>".into(),
            "<xbel><bookmark href='file:///tmp/a'></xbel>".into(),
        ] { assert!(parse_xbel(xml.as_bytes()).is_empty()); }
        assert!(parse_xbel(b"<xbel>\xff</xbel>").is_empty());
        assert_eq!(fs::read(secret).unwrap(), b"fixture-only secret");
    }
    #[test]
    fn uri_spelling_percent_decoding_and_localhost_match_python() {
        assert_eq!(
            uri_path("file://localhost/tmp/a%20b/%C3%A9?ignored#fragment"),
            Some("/tmp/a b/é".into())
        );
        assert_eq!(uri_path("file:////tmp/../a/"), Some("//tmp/../a/".into()));
        assert_eq!(
            uri_path("file:///tmp/a%FF+b%zz"),
            Some("/tmp/a�+b%zz".into())
        );
        for bad in [
            "FILE:///tmp/a",
            "file://LOCALHOST/tmp/a",
            "file://remote/tmp/a",
            "file://",
            "https://example.invalid/a",
        ] {
            assert!(uri_path(bad).is_none());
        }
    }
    #[test]
    fn recent_filters_missing_and_trash_preserves_links_and_mtime_fallback() {
        let f = Fixture::new();
        fs::write(f.root.join("note.txt"), b"fixture").unwrap();
        fs::create_dir(f.root.join("directory")).unwrap();
        symlink("note.txt", f.root.join("linked.txt")).unwrap();
        symlink("absent", f.root.join("broken")).unwrap();
        fs::create_dir_all(f.root.join("Trash/files")).unwrap();
        fs::write(f.root.join("Trash/files/hidden-history.txt"), b"fixture").unwrap();
        f.write(&format!(
            "<xbel>{}</xbel>",
            [
                "absent",
                "Trash/files/hidden-history.txt",
                "broken",
                "note.txt",
                "directory",
                "linked.txt"
            ]
            .iter()
            .map(|name| format!("<bookmark href='{}'/>", f.uri(name)))
            .collect::<String>()
        ));
        let result = recent(&f.history(), &f.root, 200);
        let rows = result.as_array().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0]["mime"], "text/plain");
        assert!(rows[0]["modified"].as_i64().unwrap() > 0);
        assert_eq!(rows[1]["isDir"], true);
        assert_eq!(rows[1]["icon"], "folder");
        assert_eq!(rows[2]["name"], "linked.txt");
        assert_eq!(
            recent(&f.history(), &f.root, 1).as_array().unwrap().len(),
            1
        );
    }
    #[test]
    fn privacy_gate_limits_and_cache_invalidation_use_only_fixture_history() {
        let f = Fixture::new();
        fs::write(f.root.join("one"), b"fixture").unwrap();
        f.write(&format!("<xbel><bookmark href='{}'/></xbel>", f.uri("one")));
        let events = Events::new(&f.root.join("events")).unwrap();
        assert_eq!(
            call_at(
                "apps.recent",
                &json!({}),
                &json!({"privacy":{"recentFiles":false}}),
                &events,
                &f.history(),
                &f.root
            )
            .unwrap(),
            json!([])
        );
        assert_eq!(
            recent(&f.history(), &f.root, 12).as_array().unwrap().len(),
            1
        );
        f.write("<xbel/>");
        assert_eq!(recent(&f.history(), &f.root, 12), json!([]));
        for (value, expected) in [
            (json!(null), 12),
            (json!(false), 1),
            (json!(true), 1),
            (json!(-8), 1),
            (json!(9.8), 9),
            (json!(999), 200),
            (json!(" +١_٩ "), 19),
            (json!("201"), 200),
            (json!("-9"), 1),
            (json!("bad"), 12),
            (json!("12.5"), 12),
            (json!("1__2"), 12),
        ] {
            assert_eq!(limit(&value), expected);
        }
        events.shutdown();
    }
    #[test]
    fn clearing_preserves_exact_malformed_bytes_and_private_modes() {
        let f = Fixture::new();
        let malformed = b"not valid XML \xff\0 but never lose these bytes";
        fs::write(f.history(), malformed).unwrap();
        fs::set_permissions(f.history(), fs::Permissions::from_mode(0o400)).unwrap();
        let backup = f.root.join("recently-used.xbel.bak");
        fs::write(&backup, b"older backup").unwrap();
        fs::set_permissions(&backup, fs::Permissions::from_mode(0o400)).unwrap();
        let result = clear(&f.history(), crate::i18n::Lang::Nl).unwrap();
        assert_eq!(result, json!({"cleared":0,"backup":backup}));
        assert_eq!(fs::read(&backup).unwrap(), malformed);
        assert_eq!(fs::read(f.history()).unwrap(), EMPTY_XBEL.as_bytes());
        for path in [f.history(), backup] {
            assert_eq!(fs::metadata(path).unwrap().mode() & 0o777, 0o600);
        }
        assert!(fs::read_dir(&f.root).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
    }
    #[test]
    fn backup_failure_keeps_source_and_does_not_publish_empty_history() {
        let f = Fixture::new();
        let raw = b"<xbel><bookmark href='file:///tmp/fixture-only'/></xbel>";
        fs::write(f.history(), raw).unwrap();
        fs::create_dir(f.root.join("recently-used.xbel.bak")).unwrap();
        let error = clear(&f.history(), crate::i18n::Lang::Nl).unwrap_err();
        assert_eq!(error.code, "failed");
        assert_eq!(error.message, "De geschiedenis kon niet worden gewist.");
        // the same failure in English (apps.historyNotCleared)
        let english = clear(&f.history(), crate::i18n::Lang::En).unwrap_err();
        assert_eq!(english.message, "Couldn’t clear the history.");
        assert_eq!(english.json()["key"], "apps.historyNotCleared");
        assert_eq!(fs::read(f.history()).unwrap(), raw);
        assert!(fs::read_dir(&f.root).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
    }
    #[test]
    fn clear_missing_file_and_symlink_match_atomic_reference_behavior() {
        let f = Fixture::new();
        assert_eq!(
            clear(&f.history(), crate::i18n::Lang::Nl).unwrap(),
            json!({"cleared":0,"backup":null})
        );
        fs::remove_file(f.history()).unwrap();
        let target = f.root.join("linked-source");
        let raw = b"<xbel><bookmark href='file:///tmp/fixture-only'/></xbel>";
        fs::write(&target, raw).unwrap();
        symlink(&target, f.history()).unwrap();
        assert_eq!(clear(&f.history(), crate::i18n::Lang::Nl).unwrap()["cleared"], 1);
        assert_eq!(fs::read(&target).unwrap(), raw);
        assert!(
            !fs::symlink_metadata(f.history())
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read(f.root.join("recently-used.xbel.bak")).unwrap(),
            raw
        );
    }
    #[test]
    fn fifo_history_cannot_block_reads_or_shutdown_and_clear_emits_event_after_commit() {
        let f = Fixture::new();
        let name = std::ffi::CString::new(f.history().as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let start = Instant::now();
        assert_eq!(recent(&f.history(), &f.root, 12), json!([]));
        assert_eq!(clear(&f.history(), crate::i18n::Lang::Nl).unwrap_err().code, "failed");
        assert!(start.elapsed().as_secs() < 1);
        fs::remove_file(f.history()).unwrap();
        f.write("<xbel/>");
        let events = Events::new(&f.root.join("events")).unwrap();
        let mut stream = events.subscribe();
        call_at(
            "apps.clearRecent",
            &json!({}),
            &json!({}),
            &events,
            &f.history(),
            &f.root,
        )
        .unwrap();
        let mut buffer = [0; 4096];
        let count = stream.read(&mut buffer).unwrap();
        assert!(
            String::from_utf8_lossy(&buffer[..count]).contains("event: apps.changed\ndata: {}")
        );
        assert_eq!(fs::read(f.history()).unwrap(), EMPTY_XBEL.as_bytes());
        events.shutdown();
    }
}
