//! On-demand video posters. Containers are decoded only by the short-lived native image worker.
use super::{ImageError, ImageResponse, decode};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex, OnceLock, Weak},
    time::Duration,
};
const FORMATS: &str = "mov,matroska,webm,avi,mpeg,mpegts,ogg,asf,flv";
pub(super) fn candidate(file: &mut File, path: &Path) -> bool {
    let mut h = [0u8; 32];
    let n = file.read(&mut h).unwrap_or(0);
    let _ = file.seek(SeekFrom::Start(0));
    let h = &h[..n];
    h.get(4..8) == Some(b"ftyp")
        || h.starts_with(b"\x1a\x45\xdf\xa3")
        || (h.get(..4) == Some(b"RIFF") && h.get(8..12) == Some(b"AVI "))
        || h.starts_with(b"OggS")
        || h.starts_with(b"FLV")
        || h.starts_with(b"\x30\x26\xb2\x75\x8e\x66\xcf\x11\xa6\xd9\x00\xaa\x00\x62\xce\x6c")
        || h.starts_with(b"\0\0\x01\xba")
        || h.starts_with(b"\0\0\x01\xb3")
        || (h.first() == Some(&0x47)
            && path.extension().is_some_and(|e| {
                ["ts", "mts", "m2ts"].contains(&e.to_string_lossy().to_lowercase().as_str())
            }))
}
pub(super) fn local_request(raw: &str) -> bool {
    if !raw.starts_with("file://") {
        return Path::new(raw).is_absolute();
    }
    url::Url::parse(raw).ok().is_some_and(|u| {
        u.host_str()
            .is_none_or(|h| h.is_empty() || h.eq_ignore_ascii_case("localhost"))
    })
}
fn gate(key: &str) -> Arc<Mutex<()>> {
    static G: OnceLock<Mutex<HashMap<String, Weak<Mutex<()>>>>> = OnceLock::new();
    let mut all = G
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(g) = all.get(key).and_then(Weak::upgrade) {
        return g;
    }
    all.retain(|_, v| v.strong_count() > 0);
    let gate = Arc::new(Mutex::new(()));
    all.insert(key.into(), Arc::downgrade(&gate));
    gate
}
pub(super) fn thumbnail(
    path: &Path,
    st: &std::fs::Metadata,
    size: i32,
    headers: &[tiny_http::Header],
    cache: &Path,
) -> Result<ImageResponse, ImageError> {
    let bucket = [64, 128, 256, 512, 1024]
        .into_iter()
        .find(|b| *b >= size)
        .unwrap_or(1024);
    let key = super::digest(
        format!(
            "{}\0{}\0{}\0{bucket}\0video-v1",
            path.to_string_lossy(),
            super::ns(st),
            st.len()
        )
        .as_bytes(),
    );
    let root = cache.join("thumbs-video-v1");
    let cached = root.join(&key[..2]).join(format!("{key}.png"));
    let gate = gate(&key);
    let _guard = gate.lock().unwrap_or_else(|e| e.into_inner());
    super::cache_state_limit(&root, false, 128 << 20);
    if let Ok((f, st)) = super::opened(&cached) {
        return Ok(super::send_file(
            &cached,
            f,
            &st,
            "image/png",
            "private, max-age=3600",
            headers,
        ));
    }
    let (data, _) = decode::isolated_video(path, bucket).ok_or_else(|| super::error(404))?;
    if super::atomic(&cached, &data).is_ok() {
        super::cache_state_limit(&root, true, 128 << 20)
    }
    Ok(super::bytes(
        data,
        "image/png",
        format!("\"{}\"", &key[..20]),
        "private, max-age=3600",
        headers,
        false,
    ))
}
fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|p| p.join(name))
        .find(|p| p.is_file())
}
fn command(exe: &Path) -> Command {
    let mut cmd = Command::new(exe);
    unsafe {
        cmd.pre_exec(|| {
            for (lim, val) in [
                (libc::RLIMIT_AS, 384u64 << 20),
                (libc::RLIMIT_CPU, 8),
                (libc::RLIMIT_FSIZE, 8 << 20),
                (libc::RLIMIT_CORE, 0),
            ] {
                let mut old = std::mem::zeroed::<libc::rlimit>();
                if libc::getrlimit(lim, &mut old) == 0 {
                    let val = (val as libc::rlim_t).min(old.rlim_max);
                    libc::setrlimit(
                        lim,
                        &libc::rlimit {
                            rlim_cur: val,
                            rlim_max: val,
                        },
                    );
                }
            }
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            Ok(())
        });
    }
    cmd
}
fn frame(exe: &Path, path: &Path, size: i32, seek: &str) -> Option<Vec<u8>> {
    let mut c = command(exe);
    c.args([
        "-v",
        "error",
        "-nostdin",
        "-threads",
        "1",
        "-filter_threads",
        "1",
        "-protocol_whitelist",
        "file,pipe",
        "-format_whitelist",
        FORMATS,
        "-ss",
        seek,
        "-i",
    ])
    .arg(path)
    .args([
        "-map",
        "0:v:0",
        "-an",
        "-sn",
        "-dn",
        "-frames:v",
        "1",
        "-vf",
    ])
    .arg(format!(
        "scale=min({size}\\,iw):min({size}\\,ih):force_original_aspect_ratio=decrease"
    ))
    .args([
        "-threads",
        "1",
        "-c:v",
        "png",
        "-compression_level",
        "6",
        "-f",
        "image2pipe",
        "pipe:1",
    ]);
    let bytes = decode::bounded(c, Duration::from_secs(5), 8 << 20)?;
    bytes.starts_with(b"\x89PNG\r\n\x1a\n").then_some(bytes)
}
pub(super) fn encode_worker(path: &Path, size: i32) -> Option<Vec<u8>> {
    if !path.is_absolute() || !path.is_file() {
        return None;
    }
    let (mut f, _) = super::opened(path).ok()?;
    if !candidate(&mut f, path) {
        return None;
    }
    drop(f);
    let exe = which("ffmpeg")?;
    let mut seek = "1";
    if let Some(probe) = which("ffprobe") {
        let mut c = command(&probe);
        c.args([
            "-v",
            "error",
            "-threads",
            "1",
            "-protocol_whitelist",
            "file,pipe",
            "-format_whitelist",
            FORMATS,
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_type:format=duration",
            "-of",
            "json",
        ])
        .arg(path);
        let bytes = decode::bounded(c, Duration::from_secs(3), 64 << 10)?;
        let info: Value = serde_json::from_slice(&bytes).ok()?;
        if !info["streams"]
            .as_array()?
            .iter()
            .any(|v| v["codec_type"] == "video")
        {
            return None;
        }
        if info["format"]["duration"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .is_some_and(|n| n < 1.2)
        {
            seek = "0";
        }
    }
    frame(&exe, path, size.clamp(8, 1024), seek).or_else(|| {
        if seek != "0" {
            frame(&exe, path, size.clamp(8, 1024), "0")
        } else {
            None
        }
    })
}
