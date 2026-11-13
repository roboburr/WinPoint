//! Original Everything format-v2 arrays and name blobs. Immutable columns are
//! read-only mappings, never materialized as a heap of per-file paths.
use serde_json::{Value, json};
use std::{
    borrow::Cow,
    collections::{HashMap, VecDeque},
    ffi::CString,
    fs::{self, File},
    io::{self, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
pub const DIR: u8 = 1;
pub const HIDDEN: u8 = 2;
pub const EXEC: u8 = 4;
pub const LINK: u8 = 8;
pub const DEL: u8 = 16;
pub const MOUNT: u8 = 32;
pub const NOSHOW: u8 = 64;
pub const ROOT: u8 = 128;
/// An index error with a catalogue text (index.tsv). The key travels inside the io::Error up to the root error of
/// the status (engine.rs), where the host renders it in its own language; Display gives this process's language
/// (logs and the plain message of the status).
#[derive(Debug)]
pub struct Fault(pub &'static str);
impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::i18n::tr(crate::i18n::current_language(), self.0))
    }
}
impl std::error::Error for Fault {}
fn fault(kind: io::ErrorKind, key: &'static str) -> io::Error {
    io::Error::new(kind, Fault(key))
}
/// The root error of the status for a failed build: a [`Fault`] keeps its key (rendered by the host), any other
/// io::Error keeps the system's text as it is.
pub fn root_error(e: &io::Error) -> crate::files::ApiError {
    match e.get_ref().and_then(|inner| inner.downcast_ref::<Fault>()) {
        Some(fault) => crate::files::ApiError::tr(
            "failed",
            crate::i18n::current_language(),
            fault.0,
            crate::i18n::NO_PARAMS,
        ),
        None => crate::files::ApiError::new("failed", &e.to_string()),
    }
}
pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
        * 1000.
}
pub fn lower(b: &[u8]) -> Vec<u8> {
    if b.is_ascii() {
        return b.to_ascii_lowercase();
    }
    let mut out = Vec::with_capacity(b.len());
    for c in String::from_utf8_lossy(b).chars() {
        let lo = c.to_lowercase().collect::<String>();
        if lo.chars().count() == 1 && lo.len() == c.len_utf8() {
            out.extend(lo.as_bytes())
        } else {
            let mut q = [0; 4];
            out.extend(c.encode_utf8(&mut q).as_bytes())
        }
    }
    out
}
pub fn fold(b: &[u8]) -> Vec<u8> {
    if b.is_ascii() {
        return b.to_vec();
    }
    let text = String::from_utf8_lossy(b);
    let normalized = glib::normalize(&text, glib::NormalizeMode::All);
    let mut out = String::new();
    for c in normalized.chars() {
        if unsafe { glib::ffi::g_unichar_combining_class(c as u32) } == 0 {
            out.push(c)
        }
    }
    let mut out = out.into_bytes();
    out.push(0);
    out.extend(b);
    out
}
pub fn name_bytes(s: &[u8]) -> Vec<u8> {
    s.iter().map(|b| if *b == b'\n' { 0 } else { *b }).collect()
}
pub fn name_string(b: &[u8]) -> String {
    String::from_utf8_lossy(
        &b.iter()
            .map(|b| if *b == 0 { b'\n' } else { *b })
            .collect::<Vec<_>>(),
    )
    .into_owned()
}
pub fn extension(name: &str, dir: bool) -> String {
    if dir {
        return String::new();
    }
    match name.rfind('.') {
        Some(p) if p > 0 && p + 1 < name.len() => name[p + 1..].to_lowercase(),
        _ => String::new(),
    }
}
pub fn category(ext: &str) -> u8 {
    for (c, words) in [
        (
            1,
            "aac ac3 aif aifc aiff amr ape au cda dts fla flac it m1a m2a m3u m3u8 m4a m4b m4p mid midi mka mod mp2 mp3 mpa ogg oga opus pls ra rmi s3m snd spx tta voc vqf w64 wav wma wv xm",
        ),
        (
            2,
            "7z ace arj bz2 cab gz gzip jar lz lz4 lzh lzma r00 r01 rar tar taz tbz tbz2 tgz tlz txz xz z zip zst zstd deb rpm iso dmg apk xpi cpio img",
        ),
        (
            3,
            "c cc chm cpp cs css csv cxx doc docm docx dot dotm dotx epub h hh hpp htm html hxx ini java js json key log lua md mht mhtml mobi nfo odp ods odt pdf php pl pot potm potx ppa ppam pps ppsm ppsx ppt pptm pptx ps py rb rst rtf sldm sldx sql tex thmx toml tsv txt vsd wpd wps wri xlam xls xlsb xlsm xlsx xlt xltm xltx xml yaml yml",
        ),
        (
            4,
            "appimage bat cmd com desktop exe msi msp ps1 run scr sh bash vbs flatpakref snap",
        ),
        (
            5,
            "ani avif bmp cr2 cr3 dng gif heic heif ico jfif jpe jpeg jpg jxl nef orf pcx png psd raf raw rw2 arw svg svgz tga tif tiff webp wmf xcf",
        ),
        (
            6,
            "3g2 3gp 3gp2 3gpp amv asf avi bik divx f4v flv hdmov m1v m2t m2ts m2v m4v mkv mov mp2v mp4 mp4v mpe mpeg mpg mpv2 mts ogm ogv qt ram rm rmvb ts vob webm wm wmv",
        ),
    ] {
        if words.split_whitespace().any(|e| e == ext) {
            return c;
        }
    }
    0
}
pub const REAL_FS: &[&str] = &[
    "ext2", "ext3", "ext4", "btrfs", "xfs", "f2fs", "zfs", "jfs", "reiserfs", "bcachefs", "nilfs2",
    "ntfs", "ntfs3", "fuseblk", "exfat", "vfat", "msdos", "hfsplus", "apfs", "udf",
];
pub const EXCLUDES: &[&str] = &[
    "/proc",
    "/sys",
    "/dev",
    "/run/user",
    "/tmp",
    "/var/tmp",
    "/snap",
    "/var/lib/snapd",
    "/var/lib/docker",
    "/var/cache",
    "/run/snapd",
    "/var/lib/containers",
    "/var/snap",
];
#[derive(Clone, Debug)]
pub struct Mount {
    pub root: String,
    pub path: String,
    pub fs: String,
    pub source: String,
}
pub fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\'
            && i + 3 < b.len()
            && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c))
        {
            out.push((b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + b[i + 3] - b'0');
            i += 4
        } else {
            out.push(b[i]);
            i += 1
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
pub fn mounts() -> Vec<Mount> {
    fs::read_to_string("/proc/self/mountinfo")
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let p = line.split_whitespace().collect::<Vec<_>>();
            let sep = p.iter().position(|s| *s == "-")?;
            if p.len() < sep + 3 || p.len() < 5 {
                return None;
            }
            Some(Mount {
                root: unescape(p[3]),
                path: unescape(p[4]),
                fs: p[sep + 1].into(),
                source: unescape(p[sep + 2]),
            })
        })
        .collect()
}
pub fn beneath(p: &str, r: &str) -> bool {
    p == r || p.starts_with(&(r.trim_end_matches('/').to_string() + "/"))
}
pub fn mount_for<'a>(p: &str, m: &'a [Mount]) -> Option<&'a Mount> {
    m.iter()
        .filter(|m| beneath(p, &m.path))
        .max_by_key(|m| m.path.len())
}
pub fn auto_roots(m: &[Mount]) -> Vec<String> {
    let mut roots = vec!["/".into()];
    for mp in m {
        if roots.contains(&mp.path)
            || !REAL_FS.contains(&mp.fs.as_str())
            || !["/home", "/run/media", "/media", "/mnt", "/srv"]
                .iter()
                .any(|p| beneath(&mp.path, p))
            || mp.root != "/" && !mp.root.is_empty() && mp.fs != "btrfs"
            || EXCLUDES.iter().any(|p| beneath(&mp.path, p))
        {
            continue;
        }
        roots.push(mp.path.clone())
    }
    roots
}
pub fn disk(source: &str) -> String {
    if !source.starts_with("/dev/") {
        return String::new();
    }
    let dev = fs::canonicalize(source).unwrap_or_else(|_| source.into());
    let name = dev.file_name().unwrap_or_default();
    let sysp = fs::canonicalize(Path::new("/sys/class/block").join(name)).unwrap_or_default();
    if sysp.join("partition").exists() {
        sysp.parent()
            .and_then(Path::file_name)
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    } else {
        name.to_string_lossy().into_owned()
    }
}
pub fn normalize(p: &str) -> String {
    let p = p.trim();
    let path = if p == "~" {
        glib::home_dir()
    } else if let Some(rest) = p.strip_prefix("~/") {
        glib::home_dir().join(rest)
    } else {
        PathBuf::from(p)
    };
    let mut norm = PathBuf::new();
    for c in path.components() {
        match c {
            std::path::Component::ParentDir => {
                norm.pop();
            }
            std::path::Component::CurDir => {}
            _ => norm.push(c.as_os_str()),
        }
    }
    norm.to_string_lossy().into_owned()
}
pub fn classify(name: &str, meta: &fs::Metadata, hidden: bool, win: bool) -> (u8, u8) {
    let mut flags = if meta.is_dir() {
        DIR
    } else if meta.file_type().is_symlink() {
        LINK
    } else {
        0
    };
    let low = name.to_lowercase();
    if hidden
        || name.starts_with('.')
        || name.ends_with('~')
        || win
            && [
                "$recycle.bin",
                "system volume information",
                "$windows.~bt",
                "$windows.~ws",
                "$winreagent",
                "$sysreset",
                "recovery",
                "config.msi",
                "msocache",
                "pagefile.sys",
                "hiberfil.sys",
                "swapfile.sys",
                "dumpstack.log.tmp",
                "dumpstack.log",
                "bootmgr",
                "bootnxt",
                "$extend",
                "found.000",
            ]
            .contains(&low.as_str())
    {
        flags |= HIDDEN
    }
    let mut cat = category(&extension(name, meta.is_dir()));
    if meta.is_dir() {
        cat = 0
    } else if !win && cat == 0 && meta.is_file() && meta.mode() & 0o111 != 0 {
        flags |= EXEC;
        cat = 4
    }
    (flags, cat)
}
struct Mapping {
    ptr: *mut libc::c_void,
    len: usize,
}
unsafe impl Send for Mapping {}
unsafe impl Sync for Mapping {}
impl Mapping {
    fn open(p: &Path, len: Option<usize>) -> io::Result<Self> {
        let file = File::open(p)?;
        let n = file.metadata()?.len() as usize;
        if let Some(v) = len {
            if v != n {
                return Err(fault(io::ErrorKind::InvalidData, "index.cache.badColumn"));
            }
        }
        if n == 0 {
            return Ok(Self {
                ptr: std::ptr::null_mut(),
                len: 0,
            });
        }
        use std::os::fd::AsRawFd;
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                n,
                libc::PROT_READ,
                libc::MAP_PRIVATE,
                file.as_raw_fd(),
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { ptr, len: n })
    }
    fn bytes(&self) -> &[u8] {
        if self.len == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(self.ptr.cast(), self.len) }
        }
    }
    fn i32(&self, g: usize) -> i32 {
        i32::from_ne_bytes(self.bytes()[g * 4..g * 4 + 4].try_into().unwrap())
    }
    fn u32(&self, g: usize) -> u32 {
        u32::from_ne_bytes(self.bytes()[g * 4..g * 4 + 4].try_into().unwrap())
    }
    fn i64(&self, g: usize) -> i64 {
        i64::from_ne_bytes(self.bytes()[g * 8..g * 8 + 8].try_into().unwrap())
    }
}
impl Drop for Mapping {
    fn drop(&mut self) {
        if self.len > 0 {
            unsafe {
                libc::munmap(self.ptr, self.len);
            }
        }
    }
}
#[derive(Clone, Debug)]
pub struct Segment {
    pub path: String,
    pub start: usize,
    pub n: usize,
    pub fs: String,
    pub dev: u64,
    pub disk: String,
    pub built: f64,
    pub errors: u64,
}
#[derive(Clone)]
struct Chunk {
    start: usize,
    n: usize,
    orig: usize,
    low: usize,
    offs: usize,
    same: bool,
}
#[derive(Clone, Debug)]
pub struct Row {
    pub parent: i32,
    pub name: Vec<u8>,
    pub size: i64,
    pub modified: i64,
    pub flags: u8,
    pub category: u8,
    pub segment: usize,
}
pub struct Data {
    pub n: usize,
    pub segments: Vec<Segment>,
    meta: Value,
    parent: Mapping,
    size: Mapping,
    mtime: Mapping,
    flags: Mapping,
    cat: Mapping,
    orig: Mapping,
    low: Mapping,
    offs: Mapping,
    order: Mapping,
    chunks: Vec<Chunk>,
    pub overlay: Vec<Row>,
    pub changes: HashMap<usize, (i64, i64, u8, u8)>,
    pub built: Option<f64>,
    pub generation: u64,
    pub cache: PathBuf,
    counts_cache: std::sync::Mutex<Option<(usize, usize, Vec<(usize, usize)>)>>,
}
impl Data {
    pub fn load(cache: &Path) -> io::Result<Self> {
        let base = cache.join("index");
        let meta: Value = serde_json::from_slice(&fs::read(base.join("meta.json"))?)?;
        if meta["version"] != 2 {
            return Err(fault(
                io::ErrorKind::InvalidData,
                "index.cache.otherVersion",
            ));
        }
        let n = meta["N"]
            .as_u64()
            .ok_or_else(|| fault(io::ErrorKind::Other, "index.cache.noLength"))?
            as usize;
        if n > i32::MAX as usize {
            return Err(fault(io::ErrorKind::Other, "index.cache.tooManyRows"));
        }
        let segments = meta["segs"]
            .as_array()
            .ok_or_else(|| fault(io::ErrorKind::Other, "index.cache.noSegments"))?
            .iter()
            .map(|s| Segment {
                path: s["path"].as_str().unwrap_or("").into(),
                start: s["start"].as_u64().unwrap_or(0) as usize,
                n: s["n"].as_u64().unwrap_or(0) as usize,
                fs: s["fs"].as_str().unwrap_or("").into(),
                dev: s["dev"].as_u64().unwrap_or(0),
                disk: s["disk"].as_str().unwrap_or("").into(),
                built: s["built"].as_f64().unwrap_or(0.),
                errors: s["errors"].as_u64().unwrap_or(0),
            })
            .collect::<Vec<_>>();
        if segments
            .iter()
            .any(|s| s.start.checked_add(s.n).is_none_or(|e| e > n))
        {
            return Err(fault(io::ErrorKind::Other, "index.cache.badSegment"));
        }
        let (mut po, mut pl, mut ko) = (0, 0, 0);
        let mut chunks = Vec::new();
        for c in meta["chunks"]
            .as_array()
            .ok_or_else(|| fault(io::ErrorKind::Other, "index.cache.noChunks"))?
        {
            let (start, cn, olen, llen) = (
                c[0].as_u64().unwrap_or(0) as usize,
                c[1].as_u64().unwrap_or(0) as usize,
                c[2].as_u64().unwrap_or(0) as usize,
                c[3].as_u64().unwrap_or(0) as usize,
            );
            if start.checked_add(cn).is_none_or(|e| e > n) || cn > 65536 {
                return Err(fault(io::ErrorKind::Other, "index.cache.badChunk"));
            }
            chunks.push(Chunk {
                start,
                n: cn,
                orig: po,
                low: if llen == 0 { po } else { pl },
                offs: ko,
                same: llen == 0,
            });
            po += olen;
            pl += llen;
            ko += cn + 1
        }
        let mut out = Self {
            n,
            segments,
            meta: meta.clone(),
            parent: Mapping::open(&base.join("parent.bin"), Some(n * 4))?,
            size: Mapping::open(&base.join("size.bin"), Some(n * 8))?,
            mtime: Mapping::open(&base.join("mtime.bin"), Some(n * 8))?,
            flags: Mapping::open(&base.join("flags.bin"), Some(n))?,
            cat: Mapping::open(&base.join("cat.bin"), Some(n))?,
            orig: Mapping::open(&base.join("names.orig"), Some(po))?,
            low: Mapping::open(&base.join("names.low"), Some(pl))?,
            offs: Mapping::open(&base.join("offs.bin"), Some(ko * 4))?,
            order: Mapping::open(&base.join("order.bin"), Some(n * 4))?,
            chunks,
            overlay: Vec::new(),
            changes: HashMap::new(),
            built: meta["built"].as_f64().or_else(|| meta["saved"].as_f64()),
            generation: 0,
            cache: cache.into(),
            counts_cache: std::sync::Mutex::new(None),
        };
        let mut remap = HashMap::new();
        if let Ok(b) = fs::read(base.join("overlay.json")) {
            if let Ok(rows) = serde_json::from_slice::<Vec<Value>>(&b) {
                for r in rows {
                    let old = n + r[0].as_u64().unwrap_or(0) as usize;
                    let parent = r[1].as_i64().unwrap_or(-1) as i32;
                    let parent = *remap.get(&(parent as usize)).unwrap_or(&parent);
                    let g = n + out.overlay.len();
                    remap.insert(old, g as i32);
                    out.overlay.push(Row {
                        parent,
                        name: r[2].as_str().unwrap_or("").as_bytes().to_vec(),
                        size: r[3].as_i64().unwrap_or(0),
                        modified: r[4].as_i64().unwrap_or(0),
                        flags: r[5].as_u64().unwrap_or(0) as u8,
                        category: r[6].as_u64().unwrap_or(0) as u8,
                        segment: r[7].as_u64().unwrap_or(0) as usize,
                    });
                }
            }
        }
        for c in &out.chunks {
            for i in 0..=c.n {
                let off = out.offs.u32(c.offs + i) as usize;
                if off
                    > if c.same {
                        out.orig.len - c.orig
                    } else {
                        out.low.len - c.low
                    }
                {
                    return Err(fault(io::ErrorKind::Other, "index.cache.badNamePosition"));
                }
            }
        }
        Ok(out)
    }
    pub fn excluded(&self) -> Vec<String> {
        self.meta["excluded"]
            .as_array()
            .map(|v| {
                v.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn len(&self) -> usize {
        self.n + self.overlay.len()
    }
    pub fn parent(&self, g: usize) -> i32 {
        if g < self.n {
            self.parent.i32(g)
        } else {
            self.overlay[g - self.n].parent
        }
    }
    pub fn fields(&self, g: usize) -> (i64, i64, u8, u8) {
        if let Some(v) = self.changes.get(&g) {
            return *v;
        }
        if g < self.n {
            (
                self.size.i64(g),
                self.mtime.i64(g),
                self.flags.bytes()[g],
                self.cat.bytes()[g],
            )
        } else {
            let r = &self.overlay[g - self.n];
            (r.size, r.modified, r.flags, r.category)
        }
    }
    pub fn name_ref(&self, g: usize, low: bool) -> Cow<'_, [u8]> {
        if g >= self.n {
            let b = &self.overlay[g - self.n].name;
            return if low {
                Cow::Owned(lower(b))
            } else {
                Cow::Borrowed(b)
            };
        }
        let i = self
            .chunks
            .partition_point(|c| c.start <= g)
            .saturating_sub(1);
        let c = &self.chunks[i];
        let local = g - c.start;
        let a = self.offs.u32(c.offs + local) as usize;
        let b = self.offs.u32(c.offs + local + 1) as usize - 1;
        let (mapping, start) = if !low || c.same {
            (&self.orig, c.orig)
        } else {
            (&self.low, c.low)
        };
        Cow::Borrowed(&mapping.bytes()[start + a..start + b])
    }
    pub fn name(&self, g: usize, low: bool) -> Vec<u8> {
        self.name_ref(g, low).into_owned()
    }
    pub fn segment(&self, g: usize) -> usize {
        if g >= self.n {
            return self.overlay[g - self.n].segment;
        }
        self.segments
            .partition_point(|s| s.start <= g)
            .saturating_sub(1)
    }
    pub fn path(&self, g: usize) -> String {
        let mut parts: Vec<String> = Vec::new();
        let mut cur = g;
        for _ in 0..4096 {
            let p = self.parent(cur);
            if p < 0 {
                let root = &self.segments[self.segment(cur)].path;
                let mut out = root.trim_end_matches('/').to_string();
                if out.is_empty() {
                    out.push('/')
                }
                for b in parts.iter().rev() {
                    if !out.ends_with('/') {
                        out.push('/')
                    }
                    out.push_str(b)
                }
                return out;
            }
            parts.push(name_string(&self.name(cur, false)));
            if p as usize >= self.len() {
                break;
            }
            cur = p as usize
        }
        String::new()
    }
    pub fn children(&self, g: usize) -> Vec<usize> {
        let s = &self.segments[self.segment(g)];
        let mut lo = s.start + 1;
        let mut hi = s.start + s.n;
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.parent.i32(mid) < g as i32 {
                lo = mid + 1
            } else {
                hi = mid
            }
        }
        let mut kids = Vec::new();
        while lo < s.start + s.n && self.parent.i32(lo) == g as i32 {
            if self.fields(lo).2 & DEL == 0 {
                kids.push(lo)
            }
            lo += 1
        }
        kids.extend(self.overlay.iter().enumerate().filter_map(|(i, r)| {
            (r.parent == g as i32 && self.fields(self.n + i).2 & DEL == 0).then_some(self.n + i)
        }));
        kids
    }
    pub fn lookup(&self, p: &str) -> Option<usize> {
        let root = self
            .segments
            .iter()
            .enumerate()
            .filter(|(_, s)| beneath(p, &s.path))
            .max_by_key(|(_, s)| s.path.len())?;
        let mut g = root.1.start;
        if p == root.1.path {
            return Some(g);
        }
        for part in p[root.1.path.trim_end_matches('/').len()..]
            .trim_start_matches('/')
            .split('/')
        {
            g = self
                .children(g)
                .into_iter()
                .find(|g| name_string(&self.name(*g, false)) == part)?
        }
        Some(g)
    }
    pub fn invalidate(&mut self) {
        *self
            .counts_cache
            .get_mut()
            .unwrap_or_else(|e| e.into_inner()) = None;
    }
    pub fn summaries(&self) -> (usize, usize, Vec<(usize, usize)>) {
        let mut cached = self.counts_cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(v) = cached.as_ref() {
            return v.clone();
        }
        let (mut files, mut folders) = (0, 0);
        let mut segments = vec![(0, 0); self.segments.len()];
        for g in 0..self.len() {
            let f = self.fields(g).2;
            if f & (DEL | NOSHOW) == 0 {
                if f & DIR != 0 {
                    folders += 1
                } else {
                    files += 1
                }
            }
            if g < self.n && f & (DEL | NOSHOW) == 0 {
                let si = self.segment(g);
                if f & DIR != 0 {
                    segments[si].1 += 1
                } else {
                    segments[si].0 += 1
                }
            }
        }
        let v = (files, folders, segments);
        *cached = Some(v.clone());
        v
    }
    pub fn counts(&self) -> (usize, usize) {
        let (f, d, _) = self.summaries();
        (f, d)
    }
    pub fn ordered(&self) -> Vec<usize> {
        let mut extras = (self.n..self.len()).collect::<Vec<_>>();
        extras.sort_by_cached_key(|g| fold(&self.name_ref(*g, true)));
        let mut result = Vec::with_capacity(self.len());
        let mut previous = 0;
        for g in extras {
            let key = fold(&self.name_ref(g, true));
            let (mut lo, mut hi) = (previous, self.n);
            while lo < hi {
                let mid = (lo + hi) / 2;
                let base = self.order.u32(mid) as usize;
                if fold(&self.name_ref(base, true)) <= key {
                    lo = mid + 1
                } else {
                    hi = mid
                }
            }
            result.extend((previous..lo).map(|rank| self.order.u32(rank) as usize));
            result.push(g);
            previous = lo;
        }
        result.extend((previous..self.n).map(|rank| self.order.u32(rank) as usize));
        result
    }
    pub fn persist(&self, excluded: &[String]) -> io::Result<()> {
        let temp = self
            .cache
            .join(format!("index.tmp-native-{}", std::process::id()));
        if temp.exists() {
            fs::remove_dir_all(&temp)?
        }
        fs::create_dir(&temp)?;
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o700))?;
        let base = self.cache.join("index");
        for name in [
            "names.orig",
            "names.low",
            "parent.bin",
            "order.bin",
            "rank.bin",
            "offs.bin",
            "lorder.bin",
        ] {
            let source = base.join(name);
            if source.exists() {
                if fs::hard_link(&source, temp.join(name)).is_err() {
                    fs::copy(source, temp.join(name))?;
                }
            }
        }
        for (name, width) in [
            ("size.bin", 8),
            ("mtime.bin", 8),
            ("flags.bin", 1),
            ("cat.bin", 1),
        ] {
            let mut file = File::create(temp.join(name))?;
            for g in 0..self.n {
                let (s, m, f, c) = self.fields(g);
                match name {
                    "size.bin" => file.write_all(&s.to_ne_bytes())?,
                    "mtime.bin" => file.write_all(&m.to_ne_bytes())?,
                    "flags.bin" => file.write_all(&[f])?,
                    _ => file.write_all(&[c])?,
                }
            }
            let _ = width;
        }
        let rows = self
            .overlay
            .iter()
            .enumerate()
            .filter_map(|(i, r)| {
                let (s, m, f, c) = self.fields(self.n + i);
                (f & DEL == 0).then(|| {
                    json!([
                        i,
                        r.parent,
                        String::from_utf8_lossy(&r.name),
                        s,
                        m,
                        f,
                        c,
                        r.segment
                    ])
                })
            })
            .collect::<Vec<_>>();
        fs::write(temp.join("overlay.json"), serde_json::to_vec(&rows)?)?;
        let mut meta = self.meta.clone();
        meta["built"] = json!(self.built);
        meta["saved"] = json!(now());
        meta["excluded"] = json!(excluded);
        write_json(&temp.join("meta.json"), &meta)?;
        install(&self.cache, &temp)
    }
}
fn write_json(p: &Path, v: &Value) -> io::Result<()> {
    let mut f = File::create(p)?;
    f.write_all(&serde_json::to_vec(v)?)?;
    f.sync_all()
}
fn install(cache: &Path, temp: &Path) -> io::Result<()> {
    let base = cache.join("index");
    let old = cache.join(format!(
        "index.old-native-{}-{}",
        std::process::id(),
        now() as u64
    ));
    let had = base.exists();
    if had {
        fs::rename(&base, &old)?
    }
    if let Err(e) = fs::rename(temp, &base) {
        if had {
            let _ = fs::rename(old, base);
        }
        return Err(e);
    }
    if had {
        let _ = fs::remove_dir_all(old);
    }
    Ok(())
}
pub fn build(
    cache: &Path,
    roots: &[String],
    excluded: &[String],
    cancel: &AtomicBool,
    progress: &dyn Fn(usize, &str),
) -> io::Result<Data> {
    let mut rows: Vec<Row> = Vec::new();
    let mut segments = Vec::new();
    let mounts = mounts();
    let exclusions = EXCLUDES
        .iter()
        .map(|s| s.to_string())
        .chain(excluded.iter().map(|s| normalize(s)))
        .collect::<Vec<_>>();
    for root in roots {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        let meta = match fs::metadata(root) {
            Ok(m) if m.is_dir() => m,
            _ => continue,
        };
        let start = rows.len();
        let mp = mount_for(root, &mounts);
        let fs_type = mp.map(|m| m.fs.clone()).unwrap_or_default();
        let win = [
            "ntfs", "ntfs3", "fuseblk", "exfat", "vfat", "msdos", "fat", "fat32",
        ]
        .contains(&fs_type.as_str());
        let si = segments.len();
        let rootpath = Path::new(root);
        let name = rootpath.file_name().map(|s| s.as_bytes()).unwrap_or(b"/");
        rows.push(Row {
            parent: -1,
            name: name_bytes(name),
            size: 0,
            modified: meta.mtime() * 1000 + meta.mtime_nsec() / 1_000_000,
            flags: DIR | ROOT,
            category: 0,
            segment: si,
        });
        let mut queue = VecDeque::from([(start, PathBuf::from(root), false)]);
        let mut errors = 0;
        while let Some((parent, path, hidden)) = queue.pop_front() {
            if cancel.load(Ordering::Relaxed) {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
            }
            progress(rows.len(), &path.to_string_lossy());
            let entries = match fs::read_dir(&path) {
                Ok(e) => e,
                Err(_) => {
                    errors += 1;
                    continue;
                }
            };
            for e in entries {
                let e = match e {
                    Ok(e) => e,
                    Err(_) => {
                        errors += 1;
                        continue;
                    }
                };
                let m = match fs::symlink_metadata(e.path()) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                let p = e.path();
                if m.is_dir()
                    && exclusions
                        .iter()
                        .any(|s| s == p.to_string_lossy().as_ref() && beneath(s, root) && s != root)
                {
                    continue;
                }
                let name = e.file_name();
                let text = name.to_string_lossy();
                let (mut flags, cat) = classify(&text, &m, hidden, win);
                let gid = rows.len();
                if m.is_dir() {
                    if m.dev() != meta.dev() {
                        flags |= MOUNT
                    } else {
                        queue.push_back((gid, p, flags & HIDDEN != 0))
                    }
                }
                rows.push(Row {
                    parent: parent as i32,
                    name: name_bytes(name.as_bytes()),
                    size: if m.is_dir() || m.file_type().is_symlink() {
                        0
                    } else {
                        m.size().min(i64::MAX as u64) as i64
                    },
                    modified: m.mtime() * 1000 + m.mtime_nsec() / 1_000_000,
                    flags,
                    category: cat,
                    segment: si,
                });
                if rows.len() > i32::MAX as usize {
                    return Err(fault(io::ErrorKind::Other, "index.cache.tooManyRows"));
                }
            }
        }
        for i in (start + 1..rows.len()).rev() {
            let p = rows[i].parent as usize;
            let size = rows[i].size;
            rows[p].size = rows[p].size.saturating_add(size)
        }
        segments.push(Segment {
            path: root.clone(),
            start,
            n: rows.len() - start,
            fs: fs_type,
            dev: meta.dev(),
            disk: mp.map(|m| disk(&m.source)).unwrap_or_default(),
            built: now(),
            errors,
        });
    }
    fs::create_dir_all(cache)?;
    let temp = cache.join(format!("index.tmp-native-{}", std::process::id()));
    if temp.exists() {
        fs::remove_dir_all(&temp)?
    }
    fs::create_dir(&temp)?;
    fs::set_permissions(&temp, fs::Permissions::from_mode(0o700))?;
    let mut fo = File::create(temp.join("names.orig"))?;
    let mut fl = File::create(temp.join("names.low"))?;
    let mut offs = File::create(temp.join("offs.bin"))?;
    let mut orderloc = File::create(temp.join("lorder.bin"))?;
    let mut chunks = Vec::new();
    for (start, chunk) in rows.chunks(65536).enumerate() {
        let start = start * 65536;
        let mut orig = vec![b'\n'];
        let mut low = vec![b'\n'];
        let mut pos = 1u32;
        let mut keys = Vec::new();
        for row in chunk {
            offs.write_all(&pos.to_ne_bytes())?;
            orig.extend(&row.name);
            orig.push(b'\n');
            let lb = lower(&row.name);
            keys.push(fold(&lb));
            low.extend(&lb);
            low.push(b'\n');
            pos = orig.len() as u32;
        }
        offs.write_all(&pos.to_ne_bytes())?;
        let mut local = (0..chunk.len()).collect::<Vec<_>>();
        local.sort_by(|a, b| keys[*a].cmp(&keys[*b]));
        for i in local {
            orderloc.write_all(&(i as u16).to_ne_bytes())?
        }
        fo.write_all(&orig)?;
        let llen = if orig == low {
            0
        } else {
            fl.write_all(&low)?;
            low.len()
        };
        chunks.push(json!([start, chunk.len(), orig.len(), llen]));
    }
    let mut order = (0..rows.len()).collect::<Vec<_>>();
    order.sort_by_cached_key(|g| fold(&lower(&rows[*g].name)));
    let mut rank = vec![0u32; rows.len()];
    let mut f = File::create(temp.join("order.bin"))?;
    for (i, g) in order.iter().enumerate() {
        f.write_all(&(*g as u32).to_ne_bytes())?;
        rank[*g] = i as u32;
    }
    let mut f = File::create(temp.join("rank.bin"))?;
    for v in rank {
        f.write_all(&v.to_ne_bytes())?
    }
    for name in [
        "parent.bin",
        "size.bin",
        "mtime.bin",
        "flags.bin",
        "cat.bin",
    ] {
        let mut f = File::create(temp.join(name))?;
        for row in &rows {
            match name {
                "parent.bin" => f.write_all(&row.parent.to_ne_bytes())?,
                "size.bin" => f.write_all(&row.size.to_ne_bytes())?,
                "mtime.bin" => f.write_all(&row.modified.to_ne_bytes())?,
                "flags.bin" => f.write_all(&[row.flags])?,
                _ => f.write_all(&[row.category])?,
            }
        }
    }
    fs::write(temp.join("overlay.json"), b"[]")?;
    let segs=segments.iter().map(|s|json!({"path":s.path,"start":s.start,"n":s.n,"fs":s.fs,"dev":s.dev,"disk":s.disk,"built":s.built,"errors":s.errors,"dead":0,"noshow":false})).collect::<Vec<_>>();
    let meta = json!({"version":2,"N":rows.len(),"saved":now(),"base_id":format!("native-{}-{}",std::process::id(),now()as u64),"segs":segs,"chunks":chunks,"built":now(),"excluded":excluded});
    write_json(&temp.join("meta.json"), &meta)?;
    drop(rows);
    install(cache, &temp)?;
    Data::load(cache)
}
#[cfg(test)]
mod tests {
    use super::*;
    // Review M1: a column file of the wrong length is a Fault like the other index file problems, so the host
    // renders its root error in its own language (index.cache.badColumn).
    #[test]
    fn a_column_of_the_wrong_length_keeps_its_key_up_to_the_root_error() {
        let dir = std::env::temp_dir().join(format!("win11-index-col-{}", crate::events::nonce()));
        fs::create_dir_all(&dir).unwrap();
        let column = dir.join("size.bin");
        fs::write(&column, [0u8; 12]).unwrap();
        assert!(Mapping::open(&column, Some(12)).is_ok());
        let e = Mapping::open(&column, Some(16)).err().unwrap();
        assert_eq!(e.kind(), io::ErrorKind::InvalidData);
        let root = root_error(&e);
        assert_eq!(
            (root.code.as_str(), root.key.as_deref()),
            ("failed", Some("index.cache.badColumn"))
        );
        assert_eq!(
            crate::i18n::tr(crate::i18n::Lang::En, "index.cache.badColumn"),
            "Inconsistent index column"
        );
        assert_eq!(
            crate::i18n::tr(crate::i18n::Lang::Nl, "index.cache.badColumn"),
            "Inconsistente indexkolom"
        );
        let _ = fs::remove_dir_all(dir);
    }
}
