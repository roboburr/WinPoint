//! What GNOME Settings' Apps panel reads about a Flatpak app without libflatpak: its deployed `metadata` keyfile
//! (the static permissions of "Vereiste machtigingen", cc-applications-panel.c add_static_permissions), the version
//! from its AppStream file, and the disk use of the deploy and of `~/.var/app/<id>` (the Storage section).
use serde_json::{Value, json};
use std::collections::HashSet;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A Flatpak application id (`org.gnome.Calculator`): at least two dots-separated parts of `[A-Za-z0-9_-]`.
pub(super) fn id_ok(id: &str) -> bool {
    let parts: Vec<&str> = id.split('.').collect();
    id.len() <= 255
        && parts.len() >= 2
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
                && !p.as_bytes()[0].is_ascii_digit()
        })
}

/// The installations flatpak itself uses: the user's, then the system's (`FLATPAK_USER_DIR`, `FLATPAK_SYSTEM_DIR`).
pub(super) fn installations() -> Vec<PathBuf> {
    let user = std::env::var_os("FLATPAK_USER_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| glib::user_data_dir().join("flatpak"));
    let system = std::env::var_os("FLATPAK_SYSTEM_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/lib/flatpak"));
    vec![user, system]
}

/// The active deploy of `id` in the first installation that has it.
pub(super) fn deploy(roots: &[PathBuf], id: &str) -> Option<PathBuf> {
    if !id_ok(id) {
        return None;
    }
    roots
        .iter()
        .map(|r| r.join("app").join(id).join("current").join("active"))
        .find(|p| p.join("metadata").is_file())
}

/// GNOME's static permission rows from a metadata keyfile: (key, access) with access "full" or "read".
pub(super) fn static_permissions(metadata: &str) -> Vec<Value> {
    let kf = glib::KeyFile::new();
    if kf
        .load_from_data(metadata, glib::KeyFileFlags::NONE)
        .is_err()
    {
        return Vec::new();
    }
    let list = |group: &str, key: &str| -> Vec<String> {
        kf.string_list(group, key)
            .map(|l| l.iter().map(|s| s.to_string()).collect())
            .unwrap_or_default()
    };
    let has = |l: &[String], v: &str| l.iter().any(|x| x == v);
    let (sockets, devices, shared, files) = (
        list("Context", "sockets"),
        list("Context", "devices"),
        list("Context", "shared"),
        list("Context", "filesystems"),
    );
    let mut out = Vec::new();
    let mut push = |key: &str, access: &str| out.push(json!({ "key": key, "access": access }));
    if has(&sockets, "system-bus") {
        push("systemBus", "full");
    }
    if has(&sockets, "session-bus") {
        push("sessionBus", "full");
    }
    if has(&devices, "all") {
        push("devices", "full");
    }
    if has(&shared, "network") {
        push("network", "full");
    }
    if has(&files, "home") || has(&files, "home:rw") || has(&files, "~") || has(&files, "~:rw") {
        push("home", "full");
    } else if has(&files, "home:ro") || has(&files, "~:ro") {
        push("home", "read");
    }
    if has(&files, "host") || has(&files, "host:rw") {
        push("fileSystem", "full");
    } else if has(&files, "host:ro") {
        push("fileSystem", "read");
    }
    if kf
        .string("Session Bus Policy", "ca.desrt.dconf")
        .is_ok_and(|v| v.as_str() == "talk")
    {
        push("settings", "full");
    }
    // Windows' "Geluid": the PulseAudio socket (GNOME shows no row for it)
    if has(&sockets, "pulseaudio") {
        push("sound", "full");
    }
    out
}

/// The newest release in an AppStream file (`<release version=...>`, the first one listed).
pub(super) fn appstream_version(xml: &str) -> Option<String> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    doc.descendants()
        .filter(|n| n.has_tag_name("release"))
        .find_map(|n| n.attribute("version"))
        .filter(|v| !v.is_empty() && v.len() <= 64)
        .map(str::to_owned)
}

/// The version of a deployed app, from its metainfo or appdata file.
pub(super) fn version(deploy: &Path, id: &str) -> Option<String> {
    let share = deploy.join("files").join("share");
    [
        share.join("metainfo").join(format!("{id}.metainfo.xml")),
        share.join("metainfo").join(format!("{id}.appdata.xml")),
        share.join("appdata").join(format!("{id}.appdata.xml")),
    ]
    .iter()
    .find_map(|p| {
        if std::fs::metadata(p).ok()?.len() > 2 << 20 {
            return None;
        }
        appstream_version(&std::fs::read_to_string(p).ok()?)
    })
}

/// A disk-use walk with limits: at most this many entries and this long, hard links counted once.
pub(super) struct Walk {
    seen: HashSet<(u64, u64)>,
    entries: usize,
    until: Instant,
    pub truncated: bool,
}

impl Walk {
    pub(super) fn new() -> Walk {
        Walk::with(Duration::from_secs(6))
    }

    pub(super) fn with(budget: Duration) -> Walk {
        Walk {
            seen: HashSet::new(),
            entries: 0,
            until: Instant::now() + budget,
            truncated: false,
        }
    }

    /// The bytes `path` occupies on disk (st_blocks), without following symlinks, leaving out `skip`.
    pub(super) fn usage(&mut self, path: &Path, skip: Option<&Path>) -> u64 {
        if skip == Some(path) {
            return 0;
        }
        if self.entries >= 300_000 || Instant::now() >= self.until {
            self.truncated = true;
            return 0;
        }
        self.entries += 1;
        let Ok(meta) = std::fs::symlink_metadata(path) else {
            return 0;
        };
        let own =
            if meta.nlink() > 1 && !meta.is_dir() && !self.seen.insert((meta.dev(), meta.ino())) {
                0
            } else {
                meta.blocks() * 512
            };
        if !meta.is_dir() {
            return own;
        }
        let Ok(dir) = std::fs::read_dir(path) else {
            return own;
        };
        let mut total = own;
        for entry in dir.flatten() {
            total += self.usage(&entry.path(), skip);
        }
        total
    }
}

/// `~/.var/app/<id>` and its cache, when they are real directories (never through a symlink).
pub(super) fn data_dirs(home: &Path, id: &str) -> Option<(PathBuf, PathBuf)> {
    if !id_ok(id) {
        return None;
    }
    let data = home.join(".var").join("app").join(id);
    let real_dir = |p: &Path| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir());
    (real_dir(&home.join(".var")) && real_dir(&home.join(".var").join("app")) && real_dir(&data))
        .then(|| (data.clone(), data.join("cache")))
}

/// Empty `cache` (its entries, not the directory itself, as GNOME's Clear Cache): the bytes freed. A symlinked cache
/// directory is left alone; entries are removed without following links.
pub(super) fn clear(cache: &Path) -> std::io::Result<u64> {
    let meta = std::fs::symlink_metadata(cache)?;
    if !meta.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "not a directory",
        ));
    }
    let freed = Walk::new()
        .usage(cache, None)
        .saturating_sub(meta.blocks() * 512);
    let mut first_error = None;
    for entry in std::fs::read_dir(cache)?.flatten() {
        let path = entry.path();
        let result = match std::fs::symlink_metadata(&path) {
            Ok(m) if m.is_dir() => std::fs::remove_dir_all(&path),
            Ok(_) => std::fs::remove_file(&path),
            Err(e) => Err(e),
        };
        if let Err(e) = result {
            first_error.get_or_insert(e);
        }
    }
    match first_error {
        Some(e) => Err(e),
        None => Ok(freed),
    }
}
