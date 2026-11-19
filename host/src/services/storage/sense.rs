//! `storage.cleanTemp` (batch D, notes/claude/settings-parity.md §5.5, amendment 21): GNOME's "Delete Temporary
//! Files…" and Windows' "Opslaginzicht nu uitvoeren" for Systeem › Opslag › Opslaginzicht.
//!
//! * `{}`: GNOME Settings' own call, gsd-housekeeping's `RemoveTempFiles` (session bus
//!   org.gnome.SettingsDaemon.Housekeeping: every file of this user in g_get_tmp_dir()). Without gsd-housekeeping
//!   (Hyprland) the host does the same itself ([`purge_temp`] with "older than now").
//! * `{sense: true}`: what gsd-housekeeping does every hour with org.gnome.desktop.privacy (gsd-disk-space.c
//!   `ldsm_purge_trash_and_temp`): temporary files older than old-files-age days when remove-old-temp-files is on,
//!   items deleted more than old-files-age days ago from every Prullenbak when remove-old-trash-files is on; 0 days
//!   is GNOME's "1 hour" (gsd then purges everything at its next hourly run, so here: everything).
//!
//! gsd's rules, as its `delete_recursively_by_age`: only this user's files, by their status change time (ctime),
//! children first, a directory only once it is empty, never across a mount point, never through a symbolic link
//! (the walk holds each directory open and goes on from that descriptor, so a path swapped meanwhile cannot redirect
//! it). A test build touches only the directories its tests name.
use crate::files::{ApiError, ApiResult};
use crate::i18n::{Lang, NO_PARAMS};
use crate::services::gsettings;
use crate::services::system::common::call;
use glib::variant::ToVariant;
use serde_json::{Value, json};
use std::ffi::{CString, OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

pub(crate) const HOUSEKEEPING: &str = "org.gnome.SettingsDaemon.Housekeeping";
pub(crate) const HOUSEKEEPING_PATH: &str = "/org/gnome/SettingsDaemon/Housekeeping";
const PRIVACY: &str = "org.gnome.desktop.privacy";
/// How deep the walk goes (gsd has no limit; a deeper tree is left for its next run).
const MAX_DEPTH: usize = 256;

/// What a purge removed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Purged {
    pub removed: u64,
    pub freed: u64,
    pub failed: u64,
}

impl Purged {
    fn add(&mut self, other: Purged) {
        self.removed += other.removed;
        self.freed += other.freed;
        self.failed += other.failed;
    }
    fn json(self) -> Value {
        json!({"removed": self.removed, "freed": self.freed, "failed": self.failed})
    }
}

fn c_name(name: &OsStr) -> io::Result<CString> {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes == b"." || bytes == b".." || bytes.contains(&b'/') {
        return Err(io::Error::from_raw_os_error(libc::EINVAL));
    }
    CString::new(bytes).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))
}

fn open_at(parent: &File, name: &OsStr, flags: i32) -> io::Result<File> {
    let name = c_name(name)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

fn open_dir_at(parent: &File, name: &OsStr) -> io::Result<File> {
    open_at(parent, name, libc::O_RDONLY | libc::O_DIRECTORY)
}

fn stat_at(parent: &File, name: &OsStr) -> io::Result<libc::stat> {
    let name = c_name(name)?;
    let mut st = std::mem::MaybeUninit::uninit();
    if unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            name.as_ptr(),
            st.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } < 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { st.assume_init() })
    }
}

fn names_at(dir: &File) -> io::Result<Vec<OsString>> {
    fs::read_dir(format!("/proc/self/fd/{}", dir.as_raw_fd()))?
        .map(|e| e.map(|e| e.file_name()))
        .collect()
}

fn unlink_at(parent: &File, name: &OsStr, directory: bool) -> io::Result<()> {
    let name = c_name(name)?;
    let flags = if directory { libc::AT_REMOVEDIR } else { 0 };
    if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), flags) } < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn is_dir(st: &libc::stat) -> bool {
    st.st_mode & libc::S_IFMT == libc::S_IFDIR
}

/// The bytes a removal frees: a regular file's size (not a link's, a socket's or a directory's).
fn file_size(st: &libc::stat) -> u64 {
    if st.st_mode & libc::S_IFMT == libc::S_IFREG {
        st.st_size.max(0) as u64
    } else {
        0
    }
}

/// A directory to walk from: its real path, opened as a directory.
fn open_root(path: &Path) -> io::Result<File> {
    let real = fs::canonicalize(path)?;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(real)
}

/// The child directory `name` of `dir`, only when it is still the one `st` describes (no swap in between).
fn same_dir(dir: &File, name: &OsStr, st: &libc::stat) -> Option<File> {
    let child = open_dir_at(dir, name).ok()?;
    let meta = child.metadata().ok()?;
    (meta.dev() == st.st_dev && meta.ino() == st.st_ino).then_some(child)
}

/// gsd's should_purge_file outside the trash: this user's, changed at or before `old`.
fn old_and_ours(st: &libc::stat, old: i64, uid: u32) -> bool {
    st.st_uid == uid && st.st_ctime <= old
}

fn walk_temp(dir: &File, dev: u64, old: i64, uid: u32, depth: usize, out: &mut Purged) {
    let Ok(names) = names_at(dir) else { return };
    for name in names {
        let Ok(st) = stat_at(dir, &name) else {
            continue;
        };
        // gsd-housekeeping: "skipping file … as it is on a different file system"
        if st.st_dev != dev {
            continue;
        }
        let directory = is_dir(&st);
        if directory && depth < MAX_DEPTH {
            if let Some(child) = same_dir(dir, &name, &st) {
                walk_temp(&child, dev, old, uid, depth + 1, out);
            }
        }
        // the status as it is now: a directory whose children went has a new ctime (gsd checks it afterwards too)
        let Ok(now) = stat_at(dir, &name) else {
            continue;
        };
        if now.st_ino != st.st_ino || !old_and_ours(&now, old, uid) {
            continue;
        }
        match unlink_at(dir, &name, directory) {
            Ok(()) => {
                out.removed += 1;
                out.freed += file_size(&now);
            }
            // a directory that still holds someone else's (or a newer) file stays, as in gsd
            Err(e) if directory && e.raw_os_error() == Some(libc::ENOTEMPTY) => {}
            Err(e) if e.raw_os_error() == Some(libc::ENOENT) => {}
            Err(_) => out.failed += 1,
        }
    }
}

/// gsd_ldsm_purge_temp_files: every entry below `root` (not `root` itself) that belongs to `uid` and changed at or
/// before `old` (seconds since the epoch), on `root`'s file system.
pub(crate) fn purge_temp(root: &Path, old: i64, uid: u32) -> io::Result<Purged> {
    let dir = open_root(root)?;
    let dev = dir.metadata()?.dev();
    let mut out = Purged::default();
    walk_temp(&dir, dev, old, uid, 0, &mut out);
    Ok(out)
}

/// A trashed item and everything in it (it is the user's own Prullenbak).
fn remove_tree(dir: &File, name: &OsStr, dev: u64, depth: usize, out: &mut Purged) -> bool {
    let Ok(st) = stat_at(dir, name) else {
        return false;
    };
    if st.st_dev != dev {
        out.failed += 1;
        return false;
    }
    if is_dir(&st) {
        if depth >= MAX_DEPTH {
            out.failed += 1;
            return false;
        }
        let Some(child) = same_dir(dir, name, &st) else {
            out.failed += 1;
            return false;
        };
        for n in names_at(&child).unwrap_or_default() {
            remove_tree(&child, &n, dev, depth + 1, out);
        }
    }
    match unlink_at(dir, name, is_dir(&st)) {
        Ok(()) => {
            out.removed += 1;
            out.freed += file_size(&st);
            true
        }
        Err(_) => {
            out.failed += 1;
            false
        }
    }
}

/// A .trashinfo's DeletionDate (local time, YYYY-MM-DDThh:mm:ss) as seconds since the epoch.
pub(crate) fn deletion_date(info: &str) -> Option<i64> {
    let value = info
        .lines()
        .find_map(|l| l.trim().strip_prefix("DeletionDate="))?
        .trim();
    let (date, time) = value.split_once('T')?;
    let d: Vec<i32> = date
        .split('-')
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    let t: Vec<i32> = time
        .get(..8)?
        .split(':')
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    if d.len() != 3 || t.len() != 3 {
        return None;
    }
    glib::DateTime::from_local(d[0], d[1], d[2], t[0], t[1], t[2] as f64)
        .ok()
        .map(|dt| dt.to_unix())
}

/// gsd_ldsm_purge_trash for one Prullenbak (a directory with `files` and `info`): every item whose DeletionDate is at
/// or before `old`, with its .trashinfo. An item without its info is left alone (it is not in the Prullenbak).
pub(crate) fn purge_trash(trash: &Path, old: i64) -> io::Result<Purged> {
    let root = open_root(trash)?;
    let info = open_dir_at(&root, OsStr::new("info"))?;
    let files = open_dir_at(&root, OsStr::new("files"))?;
    let dev = files.metadata()?.dev();
    let mut out = Purged::default();
    for name in names_at(&info)? {
        let Some(item) = name.as_bytes().strip_suffix(b".trashinfo") else {
            continue;
        };
        let Ok(file) = open_at(&info, &name, libc::O_RDONLY) else {
            continue;
        };
        let mut text = String::new();
        if file.take(64 << 10).read_to_string(&mut text).is_err() {
            continue;
        }
        let Some(deleted) = deletion_date(&text) else {
            continue;
        };
        if deleted > old {
            continue;
        }
        let item = OsStr::from_bytes(item);
        let gone = match stat_at(&files, item) {
            Ok(_) => remove_tree(&files, item, dev, 0, &mut out),
            Err(e) => e.raw_os_error() == Some(libc::ENOENT),
        };
        if gone {
            let _ = unlink_at(&info, &name, false);
        }
    }
    Ok(out)
}

fn now() -> i64 {
    glib::real_time() / 1_000_000
}

fn uid() -> u32 {
    unsafe { libc::getuid() }
}

/// g_get_tmp_dir(); a test build only what WIN11_TEST_TMPDIR names.
fn temp_root() -> Option<PathBuf> {
    if cfg!(test) {
        return std::env::var_os("WIN11_TEST_TMPDIR").map(PathBuf::from);
    }
    Some(glib::tmp_dir())
}

/// Every Prullenbak of this user (files/trash.rs's: the home one and those on mounted drives); none in a test build.
fn trash_roots() -> Vec<PathBuf> {
    if cfg!(test) {
        return Vec::new();
    }
    crate::files::storage_trash_dirs()
}

/// Whether a bus name has an owner (gsd-housekeeping runs).
fn has_owner(bus: &gio::DBusConnection, name: &str) -> bool {
    call(
        bus,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "NameHasOwner",
        Some(&(name,).to_variant()),
        1000,
    )
    .ok()
    .and_then(|r| r.child_value(0).get::<bool>())
    .unwrap_or(false)
}

/// The keys of Opslaginzicht: (remove-old-temp-files, remove-old-trash-files, old-files-age).
fn sense_keys(lang: Lang) -> (bool, bool, u32) {
    let value = |key: &str| {
        gsettings::read_item(PRIVACY, None, key, lang)
            .map(|item| item["value"].clone())
            .unwrap_or(Value::Null)
    };
    (
        value("remove-old-temp-files") == true,
        value("remove-old-trash-files") == true,
        value("old-files-age")
            .as_u64()
            .map(|n| n.min(u32::MAX as u64) as u32)
            .unwrap_or(30),
    )
}

/// Opslaginzicht nu uitvoeren: gsd's hourly purge with these keys, at `now`.
pub(crate) fn run_sense(
    temp: Option<&Path>,
    trash: &[PathBuf],
    keys: (bool, bool, u32),
    now: i64,
    uid: u32,
) -> Value {
    let (temp_on, trash_on, age) = keys;
    let old = now - age as i64 * 86_400;
    let temp_result = (temp_on)
        .then(|| temp.map(|root| purge_temp(root, old, uid).unwrap_or_default()))
        .flatten();
    let trash_result = trash_on.then(|| {
        let mut total = Purged::default();
        for root in trash {
            if let Ok(p) = purge_trash(root, old) {
                total.add(p);
            }
        }
        total
    });
    let freed = temp_result.map_or(0, |p| p.freed) + trash_result.map_or(0, |p| p.freed);
    json!({
        "ok": true,
        "age": age,
        "temp": temp_result.map(Purged::json),
        "trash": trash_result.map(Purged::json),
        "freed": freed,
    })
}

/// storage.cleanTemp (module docs). `session`: the session bus and gsd-housekeeping's name (tests: a fake).
pub(crate) fn clean_temp(
    args: &Value,
    lang: Lang,
    dry_run: bool,
    session: Option<(&gio::DBusConnection, &str)>,
) -> ApiResult {
    let sense = match args.get("sense") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(ApiError::tr("bad_args", lang, "core.badArgs", NO_PARAMS)),
    };
    if sense {
        let keys = sense_keys(lang);
        if dry_run {
            return Ok(
                json!({"ok": true, "dryRun": true, "temp": keys.0, "trash": keys.1, "age": keys.2}),
            );
        }
        return Ok(run_sense(
            temp_root().as_deref(),
            &trash_roots(),
            keys,
            now(),
            uid(),
        ));
    }
    let gsd = session.filter(|(bus, name)| has_owner(bus, name));
    if dry_run {
        return Ok(
            json!({"ok": true, "dryRun": true, "via": if gsd.is_some() { "gsd" } else { "host" }}),
        );
    }
    if let Some((bus, name)) = gsd {
        return call(
            bus,
            name,
            HOUSEKEEPING_PATH,
            HOUSEKEEPING,
            "RemoveTempFiles",
            None,
            30_000,
        )
        .map(|_| json!({"ok": true, "via": "gsd"}))
        .map_err(|_| ApiError::tr("failed", lang, "storage.tempFailed", NO_PARAMS));
    }
    let Some(root) = temp_root() else {
        return Err(ApiError::tr(
            "unsupported",
            lang,
            "storage.tempUnavailable",
            NO_PARAMS,
        ));
    };
    let purged = purge_temp(&root, now(), uid())
        .map_err(|_| ApiError::tr("failed", lang, "storage.tempFailed", NO_PARAMS))?;
    let mut out = purged.json();
    out["ok"] = json!(true);
    out["via"] = json!("host");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::storage::test_bus::{self, Object, Props, TestBus};
    use std::os::unix::fs::symlink;
    use std::sync::Arc;

    fn temp(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("win11-sense-{tag}-{}", crate::events::nonce()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn temporary_files_go_as_gsd_purges_them() {
        let root = temp("tmp");
        let outside = temp("outside");
        fs::write(outside.join("keep.txt"), "not in tmp").unwrap();
        fs::write(root.join("a.txt"), "12345").unwrap();
        fs::create_dir_all(root.join("dir/sub")).unwrap();
        fs::write(root.join("dir/sub/b.txt"), "123").unwrap();
        symlink(&outside, root.join("link")).unwrap();
        let me = uid();
        let now = now();
        // nothing is older than an hour ago: nothing goes
        let none = purge_temp(&root, now - 3600, me).unwrap();
        assert_eq!(none, Purged::default());
        assert!(root.join("a.txt").exists());
        // another user's files stay (the uid decides)
        assert_eq!(purge_temp(&root, now + 60, me + 1).unwrap().removed, 0);
        // everything up to a minute from now: all of it, children first, the link itself but not its target
        let all = purge_temp(&root, now + 60, me).unwrap();
        assert_eq!(
            (all.removed, all.freed, all.failed),
            (5, 8, 0),
            "a.txt, b.txt, sub, dir, link"
        );
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        assert!(root.exists(), "the folder itself stays");
        assert!(
            outside.join("keep.txt").exists(),
            "never through a symbolic link"
        );
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn a_directory_that_was_just_emptied_is_newer_than_the_age_and_stays() {
        // gsd checks a directory's ctime after its children went: with an age it stays for the next run
        let root = temp("age");
        fs::create_dir_all(root.join("dir")).unwrap();
        fs::write(root.join("dir/old.txt"), "x").unwrap();
        let now = now();
        let mut out = Purged::default();
        let dir = open_root(&root).unwrap();
        let dev = dir.metadata().unwrap().dev();
        // `old` between the file's ctime and the moment the directory changes: only the file is old enough
        std::thread::sleep(std::time::Duration::from_millis(1100));
        walk_temp(&dir, dev, now, uid(), 0, &mut out);
        assert_eq!(out.removed, 1);
        assert!(root.join("dir").exists() && !root.join("dir/old.txt").exists());
        let _ = fs::remove_dir_all(&root);
    }

    fn trashinfo(trash: &Path, name: &str, deleted: &str) {
        fs::write(
            trash.join("info").join(format!("{name}.trashinfo")),
            format!("[Trash Info]\nPath=/home/u/{name}\nDeletionDate={deleted}\n"),
        )
        .unwrap();
    }

    #[test]
    fn the_prullenbak_loses_what_was_deleted_long_enough_ago() {
        let trash = temp("trash");
        fs::create_dir_all(trash.join("files/old-dir/inner")).unwrap();
        fs::create_dir_all(trash.join("info")).unwrap();
        fs::write(trash.join("files/old-dir/inner/f"), "123456").unwrap();
        fs::write(trash.join("files/new.txt"), "1").unwrap();
        fs::write(trash.join("files/orphan.txt"), "1").unwrap();
        trashinfo(&trash, "old-dir", "2026-01-02T03:04:05");
        trashinfo(&trash, "new.txt", "2099-01-01T00:00:00");
        trashinfo(&trash, "gone.txt", "2026-01-01T00:00:00");
        let old = deletion_date("DeletionDate=2026-06-01T00:00:00").unwrap();
        let p = purge_trash(&trash, old).unwrap();
        assert_eq!(
            (p.removed, p.freed, p.failed),
            (3, 6, 0),
            "f, inner, old-dir"
        );
        assert!(!trash.join("files/old-dir").exists());
        assert!(!trash.join("info/old-dir.trashinfo").exists());
        assert!(
            !trash.join("info/gone.txt.trashinfo").exists(),
            "an info without its item goes too"
        );
        assert!(
            trash.join("files/new.txt").exists() && trash.join("info/new.txt.trashinfo").exists()
        );
        assert!(
            trash.join("files/orphan.txt").exists(),
            "not in the Prullenbak without its info"
        );
        let _ = fs::remove_dir_all(&trash);
    }

    #[test]
    fn deletion_dates_are_local_times() {
        let local = glib::DateTime::from_local(2026, 11, 16, 9, 30, 15.0)
            .unwrap()
            .to_unix();
        assert_eq!(
            deletion_date("[Trash Info]\nPath=/x\nDeletionDate=2026-11-16T09:30:15\n"),
            Some(local)
        );
        for bad in [
            "",
            "DeletionDate=",
            "DeletionDate=2026-11-16",
            "DeletionDate=x-y-zT1:2:3",
        ] {
            assert_eq!(deletion_date(bad), None, "{bad}");
        }
    }

    #[test]
    fn opslaginzicht_runs_each_part_only_when_its_switch_is_on() {
        let root = temp("sense");
        let trash = temp("sense-trash");
        fs::create_dir_all(trash.join("files")).unwrap();
        fs::create_dir_all(trash.join("info")).unwrap();
        fs::write(trash.join("files/x"), "12").unwrap();
        trashinfo(&trash, "x", "2020-01-01T00:00:00");
        fs::write(root.join("t"), "123").unwrap();
        let later = now() + 40 * 86_400;
        let off = run_sense(
            Some(&root),
            &[trash.clone()],
            (false, false, 30),
            later,
            uid(),
        );
        assert_eq!(
            (off["temp"].clone(), off["trash"].clone()),
            (Value::Null, Value::Null)
        );
        assert!(root.join("t").exists() && trash.join("files/x").exists());
        // 30 days: the temporary file (changed now) is not old enough at now + 29 days, the 2020 item is
        let some = run_sense(
            Some(&root),
            &[trash.clone()],
            (true, true, 30),
            now() + 29 * 86_400,
            uid(),
        );
        assert_eq!(some["temp"]["removed"], 0);
        assert_eq!(some["trash"]["removed"], 1);
        assert_eq!(some["freed"], 2);
        let all = run_sense(Some(&root), &[], (true, true, 30), later, uid());
        assert_eq!(all["temp"]["removed"], 1);
        assert_eq!(all["trash"]["removed"], 0);
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&trash);
    }

    const HOUSEKEEPING_XML: &str = r#"<interface name='org.gnome.SettingsDaemon.Housekeeping'>
<method name='EmptyTrash'/><method name='RemoveTempFiles'/></interface>"#;

    #[test]
    fn delete_temporary_files_is_gsd_housekeepings_own_call() {
        let Some(fake) = TestBus::start(
            vec![Object {
                path: HOUSEKEEPING_PATH.into(),
                xml: HOUSEKEEPING_XML.into(),
            }],
            Props::default(),
            Arc::new(|_, _, _, _| Ok(None)),
        ) else {
            eprintln!(
                "skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)"
            );
            return;
        };
        let bus = test_bus::client();
        let dry = clean_temp(&json!({}), Lang::Nl, true, Some((&bus, &fake.name))).unwrap();
        assert_eq!(
            (dry["dryRun"].clone(), dry["via"].clone()),
            (json!(true), json!("gsd"))
        );
        assert!(
            fake.calls_of("RemoveTempFiles").is_empty(),
            "a dry run sends nothing"
        );
        let done = clean_temp(&json!({}), Lang::Nl, false, Some((&bus, &fake.name))).unwrap();
        assert_eq!(done["via"], "gsd");
        assert_eq!(fake.calls_of("RemoveTempFiles").len(), 1);
        assert!(fake.calls_of("EmptyTrash").is_empty());
        assert_eq!(
            clean_temp(&json!({"sense": "yes"}), Lang::Nl, false, None)
                .unwrap_err()
                .code,
            "bad_args"
        );
        // without gsd-housekeeping, in a test build without WIN11_TEST_TMPDIR: nothing to clean, and nothing is
        let none = clean_temp(&json!({}), Lang::En, false, Some((&bus, ":1.999999")));
        if std::env::var_os("WIN11_TEST_TMPDIR").is_none() {
            assert_eq!(
                none.unwrap_err().key.as_deref(),
                Some("storage.tempUnavailable")
            );
        }
    }
}
