use super::helpers;
use glib::Variant;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};
#[link(name = "z")]
unsafe extern "C" {
    fn compressBound(n: libc::c_ulong) -> libc::c_ulong;
    fn compress2(
        out: *mut u8,
        n: *mut libc::c_ulong,
        input: *const u8,
        len: libc::c_ulong,
        level: libc::c_int,
    ) -> libc::c_int;
    fn crc32(crc: libc::c_ulong, data: *const u8, len: libc::c_uint) -> libc::c_ulong;
}
pub struct Pixmap {
    pub width: i32,
    pub height: i32,
    pub data: Vec<u8>,
}
pub fn pixmap(v: Option<&Variant>) -> Option<Pixmap> {
    let v = v?;
    if v.type_().as_str() != "a(iiay)" {
        return None;
    }
    let mut best: Option<(i32, i32, Variant, (i32, i32))> = None;
    for i in 0..v.n_children().min(32) {
        let p = v.child_value(i);
        let (w, h) = (
            p.child_value(0).get::<i32>()?,
            p.child_value(1).get::<i32>()?,
        );
        let data = p.child_value(2);
        if !(1..=1024).contains(&w)
            || !(1..=1024).contains(&h)
            || data.size() != w as usize * h as usize * 4
        {
            continue;
        }
        let rank = if w.min(h) >= 32 {
            (0, w * h)
        } else {
            (1, -w * h)
        };
        if best.as_ref().is_none_or(|b| rank < b.3) {
            best = Some((w, h, data, rank))
        }
    }
    let (w, h, data, _) = best?;
    Some(Pixmap {
        width: w,
        height: h,
        data: data.fixed_array::<u8>().ok()?.to_vec(),
    })
}
pub fn scale(p: &Pixmap) -> Pixmap {
    let f = 64. / p.width.max(p.height) as f64;
    let (w, h) = (
        (p.width as f64 * f).round_ties_even().max(1.) as i32,
        (p.height as f64 * f).round_ties_even().max(1.) as i32,
    );
    let mut rgba = Vec::with_capacity(p.data.len());
    for v in p.data.chunks_exact(4) {
        rgba.extend_from_slice(&[v[1], v[2], v[3], v[0]])
    }
    let pb = gdk_pixbuf::Pixbuf::from_bytes(
        &glib::Bytes::from_owned(rgba),
        gdk_pixbuf::Colorspace::Rgb,
        true,
        8,
        p.width,
        p.height,
        p.width * 4,
    );
    let out = pb.scale_simple(w, h, gdk_pixbuf::InterpType::Hyper);
    let rgba = if let Some(pb) = out {
        let bytes = pb.read_pixel_bytes();
        let rs = pb.rowstride() as usize;
        let mut out = Vec::with_capacity(w as usize * h as usize * 4);
        for y in 0..h as usize {
            out.extend_from_slice(&bytes.as_ref()[y * rs..y * rs + w as usize * 4])
        }
        out
    } else {
        let mut out = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let ox = (((x as f64 + 0.5) * p.width as f64 / w as f64) as i32).min(p.width - 1);
                let oy = (((y as f64 + 0.5) * p.height as f64 / h as f64) as i32).min(p.height - 1);
                let i = (oy * p.width + ox) as usize * 4;
                out.extend_from_slice(&[p.data[i + 1], p.data[i + 2], p.data[i + 3], p.data[i]])
            }
        }
        out
    };
    let mut data = Vec::with_capacity(rgba.len());
    for v in rgba.chunks_exact(4) {
        data.extend_from_slice(&[v[3], v[0], v[1], v[2]])
    }
    Pixmap {
        width: w,
        height: h,
        data,
    }
}
pub fn png(p: &Pixmap) -> Option<Vec<u8>> {
    let mut raw = Vec::with_capacity((p.width as usize * 4 + 1) * p.height as usize);
    for y in 0..p.height as usize {
        raw.push(0);
        for pixel in
            p.data[y * p.width as usize * 4..(y + 1) * p.width as usize * 4].chunks_exact(4)
        {
            raw.extend_from_slice(&[pixel[1], pixel[2], pixel[3], pixel[0]])
        }
    }
    let mut n = unsafe { compressBound(raw.len() as _) };
    let mut compressed = vec![0; n as usize];
    if unsafe {
        compress2(
            compressed.as_mut_ptr(),
            &mut n,
            raw.as_ptr(),
            raw.len() as _,
            6,
        )
    } != 0
    {
        return None;
    }
    compressed.truncate(n as usize);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(p.width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(p.height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    for (tag, data) in [
        (*b"IHDR", ihdr),
        (*b"IDAT", compressed),
        (*b"IEND", Vec::new()),
    ] {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(&tag);
        out.extend_from_slice(&data);
        let mut c = tag.to_vec();
        c.extend(data);
        out.extend_from_slice(&(unsafe { crc32(0, c.as_ptr(), c.len() as _) } as u32).to_be_bytes())
    }
    Some(out)
}
pub fn png_size(data: &[u8]) -> Option<(u32, u32)> {
    (data.len() >= 33 && data.starts_with(b"\x89PNG\r\n\x1a\n") && &data[12..16] == b"IHDR").then(
        || {
            (
                u32::from_be_bytes(data[16..20].try_into().unwrap()),
                u32::from_be_bytes(data[20..24].try_into().unwrap()),
            )
        },
    )
}
pub struct Cache {
    pub path: PathBuf,
    new_files: AtomicUsize,
}
impl Cache {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            new_files: AtomicUsize::new(0),
        }
    }
    pub fn store(&self, data: &[u8], digest: &str, ext: &str) -> Option<String> {
        fs::create_dir_all(&self.path).ok()?;
        let path = self.path.join(format!("{digest}{ext}"));
        if path.is_file() {
            let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
            unsafe { libc::utimensat(libc::AT_FDCWD, c.as_ptr(), std::ptr::null(), 0) };
            return Some(path.to_string_lossy().into_owned());
        }
        let temp = self.path.join(format!(
            ".{digest}-{}-{}.tmp",
            std::process::id(),
            self.new_files.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| {
            let mut f = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temp)?;
            f.write_all(data)?;
            fs::rename(&temp, &path)?;
            Ok::<(), std::io::Error>(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
            return None;
        }
        Some(path.to_string_lossy().into_owned())
    }
    pub fn menu_png(&self, data: &[u8]) -> Option<String> {
        let (w, h) = png_size(data)?;
        if !(1..=256).contains(&w) || !(1..=256).contains(&h) {
            return None;
        }
        self.store(data, &hash(data), ".png")
    }
    pub fn pixmap(&self, p: &Pixmap) -> Option<String> {
        let mut original = format!("{}x{}:", p.width, p.height).into_bytes();
        original.extend_from_slice(&p.data);
        let digest = hash(&original);
        let data = if p.width.max(p.height) > 64 {
            png(&scale(p))?
        } else {
            png(p)?
        };
        self.store(&data, &digest, ".png")
    }
    pub fn file(&self, path: &str) -> String {
        let Ok(mut f) = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
            .open(path)
        else {
            return path.into();
        };
        let Ok(meta) = f.metadata() else {
            return path.into();
        };
        if !meta.is_file() || meta.len() == 0 || meta.len() > 512 * 1024 {
            return path.into();
        }
        let mut data = Vec::new();
        if std::io::Read::by_ref(&mut f)
            .take(512 * 1024 + 1)
            .read_to_end(&mut data)
            .is_err()
            || data.is_empty()
            || data.len() > 512 * 1024
        {
            return path.into();
        }
        let ext = if path.to_lowercase().ends_with(".svg") {
            ".svg"
        } else {
            ".png"
        };
        self.store(&data, &hash(&data), ext)
            .unwrap_or_else(|| path.into())
    }
    pub fn icon(
        &self,
        name: Option<&str>,
        pixmap: Option<&Variant>,
        theme: &str,
    ) -> Option<String> {
        let mut name = name.unwrap_or("").trim().to_string();
        if name.starts_with('/') {
            if [".png", ".svg"]
                .iter()
                .any(|s| name.to_lowercase().ends_with(s))
                && !name.contains('\0')
                && Path::new(&name).is_file()
            {
                return Some(self.file(&name));
            }
            name.clear()
        } else if !name.is_empty() && !helpers::icon_name(&name) {
            name.clear()
        }
        if !name.is_empty() && !theme.is_empty() {
            if let Some(p) = helpers::theme_icon(theme, &name) {
                return Some(self.file(&p.to_string_lossy()));
            }
        }
        let p = self::pixmap(pixmap);
        if !name.is_empty() && (p.is_none() || icon_exists(&name)) {
            return Some(name);
        }
        if let Some(p) = p {
            return self.pixmap(&p);
        }
        (!name.is_empty()).then_some(name)
    }
    pub fn prune_due(&self) -> bool {
        self.new_files.load(Ordering::Relaxed) >= 50
            && self.new_files.swap(0, Ordering::Relaxed) >= 50
    }
    pub fn prune(&self, live: &std::collections::HashSet<String>) {
        let Ok(entries) = fs::read_dir(&self.path) else {
            return;
        };
        let mut entries = entries
            .flatten()
            .filter(|e| {
                e.path().is_file()
                    && ["png", "svg"].contains(
                        &e.path()
                            .extension()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .as_ref(),
                    )
                    && !live.contains(&e.path().to_string_lossy().into_owned())
            })
            .collect::<Vec<_>>();
        if entries.len() <= 300 {
            return;
        }
        entries.sort_by_key(|e| e.metadata().ok().and_then(|m| m.modified().ok()));
        let count = entries.len() - 300;
        for entry in entries.into_iter().take(count) {
            let _ = fs::remove_file(entry.path());
        }
    }
}
pub fn hash(data: &[u8]) -> String {
    glib::compute_checksum_for_data(glib::ChecksumType::Sha1, data)
        .map(|s| s.to_string())
        .unwrap_or_default()
}
fn icon_exists(name: &str) -> bool {
    crate::files::icon_exists(name)
}
