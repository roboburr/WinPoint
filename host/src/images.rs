//! Authenticated local image routes. No image bytes are loaded merely to stream an existing file.
use crate::services::ServiceContext;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
#[path = "images/decode.rs"]
mod decode;
#[path = "images/icons.rs"]
mod icons;
#[path = "images/mime.rs"]
mod mime;
#[path = "images/video.rs"]
mod video;
pub(crate) enum ImageBody {
    Bytes(Vec<u8>),
    File(File),
}
pub(crate) struct ImageResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: ImageBody,
}
#[derive(Debug)]
pub(crate) struct ImageError {
    pub status: u16,
    pub code: String,
    pub message: String,
}
const CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src data:; sandbox";
fn error(status: u16) -> ImageError {
    ImageError {
        status,
        code: if status == 403 {
            "forbidden"
        } else {
            "not_found"
        }
        .into(),
        message: if status == 403 {
            "Forbidden"
        } else {
            "Not Found"
        }
        .into(),
    }
}
fn digest(data: &[u8]) -> String {
    glib::compute_checksum_for_data(glib::ChecksumType::Sha1, data)
        .unwrap()
        .to_string()
}
fn ns(st: &std::fs::Metadata) -> i128 {
    st.mtime() as i128 * 1_000_000_000 + st.mtime_nsec() as i128
}
fn matches(headers: &[tiny_http::Header], tag: &str) -> bool {
    headers
        .iter()
        .find(|h| h.field.equiv("If-None-Match"))
        .is_some_and(|h| h.value.as_str().split(',').any(|s| s.trim() == tag))
}
fn not_modified(tag: String) -> ImageResponse {
    ImageResponse {
        status: 304,
        headers: vec![
            ("ETag".into(), tag),
            ("Cache-Control".into(), "private, no-cache".into()),
        ],
        body: ImageBody::Bytes(vec![]),
    }
}
fn bytes(
    data: Vec<u8>,
    mime: &str,
    tag: String,
    cache: &str,
    headers: &[tiny_http::Header],
    conditional: bool,
) -> ImageResponse {
    if conditional && matches(headers, &tag) {
        return not_modified(tag);
    }
    ImageResponse {
        status: 200,
        headers: vec![
            ("Content-Type".into(), mime.into()),
            ("Content-Length".into(), data.len().to_string()),
            ("Cache-Control".into(), cache.into()),
            ("Content-Security-Policy".into(), CSP.into()),
            ("ETag".into(), tag),
        ],
        body: ImageBody::Bytes(data),
    }
}
fn opened(path: &Path) -> Result<(File, std::fs::Metadata), ImageError> {
    let f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| error(404))?;
    let st = f.metadata().map_err(|_| error(404))?;
    if !st.is_file() {
        return Err(error(404));
    }
    Ok((f, st))
}
fn send_file(
    path: &Path,
    file: File,
    st: &std::fs::Metadata,
    mime: &str,
    cache: &str,
    headers: &[tiny_http::Header],
) -> ImageResponse {
    let tag = format!(
        "\"{}\"",
        &digest(format!("{}|{}|{}", path.to_string_lossy(), ns(st), st.len()).as_bytes())[..20]
    );
    if matches(headers, &tag) {
        return not_modified(tag);
    }
    ImageResponse {
        status: 200,
        headers: vec![
            ("Content-Type".into(), mime.into()),
            ("Content-Length".into(), st.len().to_string()),
            ("Cache-Control".into(), cache.into()),
            ("Content-Security-Policy".into(), CSP.into()),
            ("ETag".into(), tag),
        ],
        body: ImageBody::File(file),
    }
}
fn system(path: &Path) -> bool {
    path.canonicalize().ok().is_some_and(|p| {
        [
            "/usr/share/",
            "/usr/local/share/",
            "/var/lib/flatpak/exports/",
            "/var/lib/snapd/desktop/",
            "/snap/",
        ]
        .iter()
        .any(|prefix| p.to_string_lossy().starts_with(prefix))
    })
}
type ConversionKey = (PathBuf, i128, u64, i32);
#[derive(Default)]
struct Conversions {
    entries: VecDeque<(ConversionKey, Arc<Vec<u8>>)>,
    bytes: usize,
}
fn converted(path: &Path, st: &std::fs::Metadata, mime: &str, size: i32) -> Option<Vec<u8>> {
    static C: OnceLock<Mutex<Conversions>> = OnceLock::new();
    let c = C.get_or_init(|| Mutex::new(Conversions::default()));
    let key = (path.into(), ns(st), st.len(), size);
    {
        let mut c = c.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = c.entries.iter().position(|(k, _)| k == &key) {
            let x = c.entries.remove(i).unwrap();
            let data = x.1.as_ref().clone();
            c.entries.push_back(x);
            return Some(data);
        }
    }
    let data = if !system(path)
        || !(mime::in_process(mime) || ["image/x-xpixmap", "image/x-xpm"].contains(&mime))
    {
        decode::isolated(path, size, false)?.0
    } else {
        let _p = decode::permit();
        decode::encode(path, size, false, false)?.0
    };
    if data.len() <= 8 << 20 {
        let mut c = c.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = c.entries.iter().position(|(k, _)| k == &key) {
            if let Some((_, v)) = c.entries.remove(i) {
                c.bytes -= v.len()
            }
        }
        c.bytes += data.len();
        c.entries.push_back((key, Arc::new(data.clone())));
        while c.bytes > 64 << 20 || c.entries.len() > 256 {
            if let Some((_, v)) = c.entries.pop_front() {
                c.bytes -= v.len()
            } else {
                break;
            }
        }
    }
    Some(data)
}
fn image(
    path: &Path,
    headers: &[tiny_http::Header],
    size: i32,
    convert: bool,
    user_icon: bool,
) -> Result<ImageResponse, ImageError> {
    let (mut f, st) = opened(path)?;
    let mime = mime::detect(&mut f, path).ok_or_else(|| error(404))?;
    let svgz = path.to_string_lossy().to_lowercase().ends_with(".svgz");
    let mut convert = convert;
    if user_icon {
        if st.len() > 2 << 20 {
            return Err(error(404));
        }
        let mut h = [0u8; 32];
        let n = f.read(&mut h).map_err(|_| error(404))?;
        f.seek(SeekFrom::Start(0)).map_err(|_| error(404))?;
        let small =
            mime::dimensions(&h[..n]).is_some_and(|(w, h)| w.max(h) <= 1024) && mime::browser(mime);
        if !(mime == "image/svg+xml" && !svgz || small) {
            convert = true
        }
    }
    if !convert && mime::browser(mime) && !svgz {
        return Ok(send_file(path, f, &st, mime, "private, no-cache", headers));
    }
    drop(f);
    let data = converted(path, &st, mime, size).ok_or_else(|| error(404))?;
    let tag = format!("\"{}\"", &digest(&data)[..20]);
    Ok(bytes(
        data,
        "image/png",
        tag,
        "private, no-cache",
        headers,
        true,
    ))
}
// urllib.parse.urlsplit(...).path + unquote(), including remote file authorities.
fn uri_path(raw: &str) -> Option<PathBuf> {
    let rest = raw.strip_prefix("file://")?;
    let start = if rest.starts_with('/') {
        0
    } else {
        rest.find('/').unwrap_or(rest.len())
    };
    let path = rest[start..].split(['?', '#']).next().unwrap_or("");
    let mut out = Vec::with_capacity(path.len());
    let bytes = path.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| (b as char).to_digit(16);
            if let (Some(a), Some(b)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((a * 16 + b) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    Some(PathBuf::from(String::from_utf8_lossy(&out).as_ref()))
}
fn absolute(raw: &str) -> Option<PathBuf> {
    if raw.is_empty() || raw.contains('\0') {
        return None;
    }
    let p = if raw.starts_with("file://") {
        uri_path(raw)?
    } else {
        raw.into()
    };
    if !p.is_absolute() {
        return None;
    }
    p.canonicalize().ok()
}
fn size(value: Option<&String>, default: i32) -> i32 {
    let Some(s) = value else { return default };
    let Ok(n) = s.trim().parse::<f64>() else {
        return default;
    };
    if !n.is_finite() {
        return default;
    }
    n.trunc().clamp(8., 1024.) as i32
}
fn fdo(path: &Path, size: i32, st: &std::fs::Metadata) -> Option<PathBuf> {
    use gio::prelude::*;
    let uri = gio::File::for_path(path).uri();
    let key = glib::compute_checksum_for_data(glib::ChecksumType::Md5, uri.as_bytes())?;
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| "/".into())
                .join(".cache")
        })
        .join("thumbnails");
    let order = if size <= 128 {
        ["normal", "large", "x-large", "xx-large"]
    } else if size <= 256 {
        ["large", "x-large", "xx-large", "normal"]
    } else {
        ["x-large", "xx-large", "large", "normal"]
    };
    order
        .into_iter()
        .map(|s| base.join(s).join(format!("{key}.png")))
        .find(|p| p.metadata().ok().is_some_and(|s| ns(&s) >= ns(st)))
}
fn atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("parent"))?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)?;
    static COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let temp = parent.join(format!(
        ".{}.{}.tmp",
        std::process::id(),
        COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let result = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        f.write_all(data)?;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        f.sync_all()?;
        std::fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}
fn cache_state(root: &Path, written: bool) {
    cache_state_limit(root, written, 512 << 20);
}
fn cache_state_limit(root: &Path, written: bool, max_bytes: u64) {
    static STATE: OnceLock<Mutex<HashMap<PathBuf, u64>>> = OnceLock::new();
    let mut prune = false;
    {
        let mut state = STATE
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if !state.contains_key(root) {
            state.insert(root.into(), 0);
            prune = true;
        } else if written {
            let n = state.get_mut(root).unwrap();
            *n += 1;
            if *n % if max_bytes < 512 << 20 { 32 } else { 500 } == 0 {
                prune = true;
            }
        }
    }
    if prune {
        let root = root.to_path_buf();
        let delay = if written { 0 } else { 30 };
        std::thread::spawn(move || {
            if delay > 0 {
                std::thread::sleep(Duration::from_secs(delay));
            }
            let _ = prune_cache(&root, max_bytes, 60. * 86400., None);
        });
    }
}
/// Remove only regular, non-symlink files exactly two levels below this cache root.
pub(crate) fn prune_cache(root: &Path, max: u64, age: f64, now: Option<f64>) -> (usize, u64) {
    let now = now.unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    });
    let mut entries = Vec::new();
    let Ok(dirs) = std::fs::read_dir(root) else {
        return (0, 0);
    };
    for sub in dirs.flatten() {
        if sub.file_name().to_string_lossy().chars().count() != 2
            || !sub.file_type().is_ok_and(|t| t.is_dir())
        {
            continue;
        }
        let Ok(files) = std::fs::read_dir(sub.path()) else {
            continue;
        };
        for e in files.flatten() {
            if !e.file_type().is_ok_and(|t| t.is_file()) {
                continue;
            }
            if let Ok(st) = e.metadata() {
                let used = (st.atime() as f64 + st.atime_nsec() as f64 / 1e9)
                    .max(st.mtime() as f64 + st.mtime_nsec() as f64 / 1e9);
                entries.push((
                    used,
                    if st.blocks() > 0 {
                        st.blocks() * 512
                    } else {
                        st.len()
                    },
                    e.path(),
                ));
            }
        }
    }
    let (mut removed, mut freed) = (0, 0);
    let mut keep = Vec::new();
    for (used, n, p) in entries {
        if now - used > age {
            if std::fs::remove_file(&p).is_ok() {
                removed += 1;
                freed += n;
            }
        } else {
            keep.push((used, n, p))
        }
    }
    let mut total = keep.iter().map(|e| e.1).sum::<u64>();
    if total > max {
        keep.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        for (_, n, p) in keep {
            if total as f64 <= max as f64 * 0.9 {
                break;
            }
            if std::fs::remove_file(p).is_ok() {
                removed += 1;
                freed += n;
                total -= n;
            }
        }
    }
    (removed, freed)
}
fn thumb(
    path: &Path,
    size: i32,
    headers: &[tiny_http::Header],
    cache: &Path,
    allow_video: bool,
) -> Result<ImageResponse, ImageError> {
    let (mut f, st) = opened(path)?;
    let mime = mime::detect(&mut f, path);
    let Some(mime) = mime else {
        let is_video = video::candidate(&mut f, path);
        if let Some(cached) = fdo(path, size, &st) {
            return image(&cached, headers, 0, false, false);
        }
        return if is_video && allow_video {
            video::thumbnail(path, &st, size, headers, cache)
        } else {
            Err(error(404))
        };
    };
    if mime == "image/svg+xml" && !path.to_string_lossy().to_lowercase().ends_with(".svgz") {
        return image(path, headers, 0, false, false);
    }
    let bucket = [64, 128, 256, 512, 1024]
        .into_iter()
        .find(|b| *b >= size)
        .unwrap_or(1024);
    let key = digest(
        format!(
            "{}\0{}\0{}\0{}",
            path.to_string_lossy(),
            ns(&st),
            st.len(),
            bucket
        )
        .as_bytes(),
    );
    let root = cache.join("thumbs");
    cache_state(&root, false);
    for (ext, m) in [("jpg", "image/jpeg"), ("png", "image/png")] {
        let p = root.join(&key[..2]).join(format!("{key}.{ext}"));
        if let Ok((f, st)) = opened(&p) {
            return Ok(send_file(&p, f, &st, m, "private, max-age=3600", headers));
        }
    }
    if st.len() > 256 << 20 {
        if mime::browser(mime) && st.len() < 12 << 20 {
            return image(path, headers, 0, false, false);
        }
        return Err(error(404));
    }
    drop(f);
    let (data, m) = if !mime::in_process(mime) {
        decode::isolated(path, bucket, true).ok_or_else(|| error(404))?
    } else {
        let p = decode::permit();
        let info = gdk_pixbuf::Pixbuf::file_info(path);
        if info.is_some_and(|(_, w, h)| w > 0 && h > 0 && w.max(h) <= bucket)
            && mime::browser(mime)
            && st.len() < 4 << 20
        {
            drop(p);
            return image(path, headers, 0, false, false);
        }
        decode::encode(path, bucket, true, true).ok_or_else(|| error(404))?
    };
    let ext = if m == "image/jpeg" { "jpg" } else { "png" };
    if atomic(&root.join(&key[..2]).join(format!("{key}.{ext}")), &data).is_ok() {
        cache_state(&root, true)
    } // As in the original server, a newly generated response is sent even with If-None-Match.
    Ok(bytes(
        data,
        m,
        format!("\"{}\"", &key[..20]),
        "private, max-age=3600",
        headers,
        false,
    ))
}
pub(crate) fn handle(
    path: &str,
    query: &url::Url,
    headers: &[tiny_http::Header],
    ctx: &Arc<ServiceContext>,
) -> Option<Result<ImageResponse, ImageError>> {
    if !["/icon", "/thumb", "/file"].contains(&path) {
        return None;
    }
    let q = query
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect::<HashMap<_, _>>();
    let get = |k: &str| q.get(k).map(String::as_str).unwrap_or("");
    Some((|| match path {
        "/icon" => {
            let size = size(q.get("size"), 32);
            let name = get("name").trim();
            let mime = get("mime").trim();
            let fallback = get("fallback").trim();
            let mut asked = name;
            let p = if !name.is_empty() {
                icons::find(name, size as i64)
            } else if !mime.is_empty() {
                icons::find_mime(mime, size as i64)
            } else {
                None
            };
            let p = p
                .or_else(|| {
                    if !fallback.is_empty() {
                        asked = fallback;
                        icons::find(fallback, size as i64)
                    } else {
                        None
                    }
                })
                .ok_or_else(|| error(404))?;
            let user = (asked.starts_with('/') || asked.starts_with("file://")) && !system(&p);
            image(&p, headers, size, false, user)
        }
        "/thumb" => {
            let p = absolute(get("path")).ok_or_else(|| error(404))?;
            thumb(
                &p,
                size(q.get("size"), 256),
                headers,
                &ctx.cache_dir,
                q.get("video").is_some_and(|v| v == "1") && video::local_request(get("path")),
            )
        }
        "/file" => {
            let raw = get("path");
            if let Some(id) = raw.strip_prefix("builtin:") {
                if !id.is_empty()
                    && id.len() <= 64
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                {
                    let id = crate::wallpaper::builtin_id(id);
                    return Ok(ImageResponse {
                        status: 302,
                        headers: vec![
                            ("Location".into(), format!("/ui/assets/wallpapers/{id}.svg")),
                            ("Content-Length".into(), "0".into()),
                            ("Cache-Control".into(), "no-store".into()),
                        ],
                        body: ImageBody::Bytes(vec![]),
                    });
                }
                return Err(error(404));
            }
            let p = absolute(raw).ok_or_else(|| error(404))?;
            if !p.is_file() {
                return Err(error(403));
            }
            let (mut f, _) = opened(&p)?;
            if mime::detect(&mut f, &p).is_none() {
                return Err(error(403));
            }
            image(&p, headers, 0, false, false)
        }
        _ => unreachable!(),
    })())
}
pub(crate) fn worker_main() -> i32 {
    decode::worker_main()
}
