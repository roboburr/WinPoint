use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};
pub(super) fn sniff(h: &[u8], ext: &str) -> Option<&'static str> {
    if h.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if h.starts_with(b"\xff\xd8\xff") {
        return Some("image/jpeg");
    }
    if h.starts_with(b"GIF87a") || h.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if h.get(..4) == Some(b"RIFF") && h.get(8..12) == Some(b"WEBP") {
        return Some("image/webp");
    }
    if h.starts_with(b"BM") && ext == "bmp" {
        return Some("image/bmp");
    }
    if h.starts_with(b"\x00\x00\x01\x00") && ["ico", "cur", ""].contains(&ext) {
        return Some("image/vnd.microsoft.icon");
    }
    if h.get(4..8) == Some(b"ftyp") {
        match h.get(8..12) {
            Some(b"avif" | b"avis") => return Some("image/avif"),
            Some(b"heic" | b"heix" | b"mif1" | b"msf1" | b"hevc") => return Some("image/heic"),
            _ => {}
        }
    }
    if h.starts_with(b"II*\0") || h.starts_with(b"MM\0*") {
        return Some("image/tiff");
    }
    if h.starts_with(b"\xff\x0a") || h.starts_with(b"\0\0\0\x0cJXL ") {
        return Some("image/jxl");
    }
    if h.starts_with(b"\x1f\x8b") && ext == "svgz" {
        return Some("image/svg+xml");
    }
    if h[..h.len().min(64)].windows(9).any(|w| w == b"/* XPM */") {
        return Some("image/x-xpixmap");
    }
    // Python bytes.lstrip treats the UTF-8 BOM as three individual bytes.
    let text = h
        .iter()
        .position(|b| !b"\xef\xbb\xbf \t\r\n".contains(b))
        .map(|i| h[i..].to_ascii_lowercase())
        .unwrap_or_default();
    if text.starts_with(b"<") && text.windows(4).any(|w| w == b"<svg") {
        return Some("image/svg+xml");
    }
    if ext == "tga" && h.len() > 18 {
        return Some("image/x-tga");
    }
    None
}
pub(super) fn detect(f: &mut File, path: &Path) -> Option<&'static str> {
    let mut h = [0u8; 4096];
    let n = f.read(&mut h).ok()?;
    f.seek(SeekFrom::Start(0)).ok()?;
    sniff(
        &h[..n],
        &path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase(),
    )
}
pub(super) fn image(path: &Path) -> bool {
    File::open(path)
        .ok()
        .is_some_and(|mut f| detect(&mut f, path).is_some())
}
pub(super) fn browser(m: &str) -> bool {
    matches!(
        m,
        "image/png"
            | "image/jpeg"
            | "image/gif"
            | "image/webp"
            | "image/svg+xml"
            | "image/bmp"
            | "image/vnd.microsoft.icon"
            | "image/avif"
    )
}
pub(super) fn in_process(m: &str) -> bool {
    matches!(m, "image/png" | "image/jpeg" | "image/gif" | "image/webp")
}
pub(super) fn dimensions(h: &[u8]) -> Option<(u32, u32)> {
    if h.len() >= 24 && h.starts_with(b"\x89PNG\r\n\x1a\n") && &h[12..16] == b"IHDR" {
        Some((
            u32::from_be_bytes(h[16..20].try_into().ok()?),
            u32::from_be_bytes(h[20..24].try_into().ok()?),
        ))
    } else if h.len() >= 10 && (h.starts_with(b"GIF87a") || h.starts_with(b"GIF89a")) {
        Some((
            u16::from_le_bytes(h[6..8].try_into().ok()?) as u32,
            u16::from_le_bytes(h[8..10].try_into().ok()?) as u32,
        ))
    } else {
        None
    }
}
