//! Names of the own desktop entries in the current Win11 language (contracts C6, decision D-G4).
//!
//! `scripts/install.py` writes the nine `io.github.win11shell.<Name>.desktop` files; GNOME's own surfaces (the
//! overview, the app grid, "… is ready" banners) show their plain `Name=`. Only win11-panel rewrites them
//! (bin/panel.rs → `enable_desktop_entries`), as `win11-panel --http-only`, the backend of win11-shell (D-G70): at
//! start and after a language switch, debounced. It changes only the
//! `Name=` and `Comment=` lines of the `[Desktop Entry]` group, only in files under `$XDG_DATA_HOME/applications`
//! that are regular files of this user whose `StartupWMClass` is `io.github.win11shell.<Name>`, atomically
//! (temporary file + rename, the mode kept) and only when the text differs. Every other byte stays as it is.
//! `update-desktop-database` is not needed: it caches MIME types, which do not change here.
use crate::i18n::Lang;
use serde_json::Value;
use std::{
    fs,
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

const TABLE: &str = include_str!("desktop-entries.json");
const LIMIT: u64 = 64 * 1024;

pub(crate) struct Entry {
    pub name: String,
    pub title: [String; 2],
}
pub(crate) struct Table {
    pub comment: [String; 2],
    pub entries: Vec<Entry>,
}

fn pair(value: &Value) -> [String; 2] {
    [
        value["nl"].as_str().unwrap_or_default().to_owned(),
        value["en"].as_str().unwrap_or_default().to_owned(),
    ]
}

/// host/src/desktop-entries.json, the table `scripts/install.py` reads too.
pub(crate) fn table() -> &'static Table {
    static TABLE_DATA: OnceLock<Table> = OnceLock::new();
    TABLE_DATA.get_or_init(|| {
        let data: Value = serde_json::from_str(TABLE).expect("embedded desktop-entries.json");
        Table {
            comment: pair(&data["comment"]),
            entries: data["entries"]
                .as_object()
                .map(|entries| {
                    entries
                        .iter()
                        .map(|(name, value)| Entry {
                            name: name.clone(),
                            title: pair(value),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    })
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Report {
    pub updated: usize,
    pub unchanged: usize,
    pub skipped: usize,
    pub failed: Vec<String>,
}

/// A desktop-entry string value (spec "Possible value types": \s for a leading space, \n \t \r \\).
fn escape_value(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, c) in text.chars().enumerate() {
        match c {
            ' ' if i == 0 => out.push_str("\\s"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out
}

/// The entry text with `Name=`/`Comment=` of its `[Desktop Entry]` group set, or None when the file is not one
/// of ours (no matching `StartupWMClass` in that group, or no `Name=` line at all).
fn rewrite(text: &str, name: &str, title: &str, comment: &str) -> Option<String> {
    let class = format!("io.github.win11shell.{name}");
    let mut group = String::new();
    let mut ours = false;
    let mut has_name = false;
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.starts_with('[') {
            group = line.trim().to_owned();
            continue;
        }
        if group == "[Desktop Entry]"
            && let Some((key, value)) = line.split_once('=')
        {
            match key.trim() {
                "StartupWMClass" if value.trim() == class => ours = true,
                "Name" => has_name = true,
                _ => {}
            }
        }
    }
    if !ours || !has_name {
        return None;
    }
    let mut out = String::with_capacity(text.len() + 64);
    let mut group = String::new();
    for piece in text.split_inclusive('\n') {
        let (line, end) = match piece.strip_suffix('\n') {
            Some(line) => match line.strip_suffix('\r') {
                Some(line) => (line, "\r\n"),
                None => (line, "\n"),
            },
            None => (piece, ""),
        };
        if line.starts_with('[') {
            group = line.trim().to_owned();
        } else if group == "[Desktop Entry]"
            && let Some((key, _)) = line.split_once('=')
        {
            let key = key.trim();
            if key == "Name" || key == "Comment" {
                let value = if key == "Name" { title } else { comment };
                out.push_str(key);
                out.push('=');
                out.push_str(&escape_value(value));
                out.push_str(end);
                continue;
            }
        }
        out.push_str(line);
        out.push_str(end);
    }
    Some(out)
}

enum Outcome {
    Updated,
    Unchanged,
    Skipped,
}

fn sync_one(dir: &Path, entry: &Entry, comment: &str, lang: Lang) -> Result<Outcome, String> {
    let path = dir.join(format!("io.github.win11shell.{}.desktop", entry.name));
    let Ok(meta) = fs::symlink_metadata(&path) else {
        return Ok(Outcome::Skipped);
    };
    if !meta.file_type().is_file()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.len() > LIMIT
    {
        return Ok(Outcome::Skipped);
    }
    let Ok(bytes) = fs::read(&path) else {
        return Ok(Outcome::Skipped);
    };
    let Ok(text) = String::from_utf8(bytes) else {
        return Ok(Outcome::Skipped);
    };
    let index = usize::from(lang == Lang::En);
    let Some(new) = rewrite(&text, &entry.name, &entry.title[index], comment) else {
        return Ok(Outcome::Skipped);
    };
    if new == text {
        return Ok(Outcome::Unchanged);
    }
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let temporary = dir.join(format!(
        ".io.github.win11shell.{}.desktop.{}-{}.tmp",
        entry.name,
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temporary)?;
        file.write_all(new.as_bytes())?;
        file.set_permissions(fs::Permissions::from_mode(meta.mode() & 0o7777))?;
        file.sync_all()?;
        fs::rename(&temporary, &path)
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(format!("{}: {error}", path.display()));
    }
    Ok(Outcome::Updated)
}

/// Rewrites the own entries in `dir` for `lang`.
pub(crate) fn sync_dir(dir: &Path, lang: Lang) -> Report {
    let table = table();
    let comment = &table.comment[usize::from(lang == Lang::En)];
    let mut report = Report::default();
    for entry in &table.entries {
        match sync_one(dir, entry, comment, lang) {
            Ok(Outcome::Updated) => report.updated += 1,
            Ok(Outcome::Unchanged) => report.unchanged += 1,
            Ok(Outcome::Skipped) => report.skipped += 1,
            Err(error) => report.failed.push(error),
        }
    }
    report
}

/// `$XDG_DATA_HOME/applications`, where install.py puts the entries.
pub(crate) fn applications_dir() -> PathBuf {
    glib::user_data_dir().join("applications")
}

static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Rewrites the entries after `delay` on a worker thread, in the language `language()` gives at that moment: the
/// settings file then, not the language of the switch that scheduled it (two quick switches may be seen out of
/// order, review m5). A newer call before then replaces it (a quick nl → en → nl writes once, or not at all).
pub(crate) fn schedule(delay: Duration, language: impl FnOnce() -> Lang + Send + 'static) {
    let _ = schedule_into(applications_dir, delay, language);
}

/// [`schedule`] for the directory `dir()` gives at run time; the worker's handle (tests join it). The result is
/// None when the worker ran nothing because a newer call replaced it.
fn schedule_into(
    dir: impl FnOnce() -> PathBuf + Send + 'static,
    delay: Duration,
    language: impl FnOnce() -> Lang + Send + 'static,
) -> Option<std::thread::JoinHandle<Option<Report>>> {
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    std::thread::Builder::new()
        .name("desktop-entries".into())
        .spawn(move || {
            std::thread::sleep(delay);
            if GENERATION.load(Ordering::SeqCst) != generation {
                return None;
            }
            let report = sync_dir(&dir(), language());
            for error in &report.failed {
                eprintln!("Win11 desktop entry {error}");
            }
            Some(report)
        })
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("win11-desktop-entries-{}", crate::events::nonce()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    // The bytes scripts/install.py writes at 3b898e6 (install.py:110-131) for a given prefix.
    fn installed(name: &str, title: &str) -> String {
        let (binary, icon, extra, args) = match name {
            "Panel" => ("win11-panel", "view-app-grid-symbolic", "NoDisplay=true\nCategories=Utility;\n", ""),
            "Settings" => ("win11-app", "/p/share/win11-gnome/ui/assets/icons/settings.svg", "NoDisplay=true\nCategories=Settings;\n", " --app settings"),
            "DefaultApps" => ("win11-default-apps", "/p/share/win11-gnome/ui/assets/icons/settings.svg", "Categories=Settings;System;\n", ""),
            "Installer" => ("win11-installer", "system-software-install", "Categories=System;PackageManager;\n", ""),
            "Search" => ("win11-app", "system-search", "NoDisplay=true\nCategories=Utility;\n", " --app alles"),
            "Disks" => ("win11-app", "drive-harddisk", "NoDisplay=true\nCategories=System;\n", " --app disks"),
            "TaskManager" => ("win11-app", "utilities-system-monitor", "NoDisplay=true\nCategories=System;Monitor;\n", " --app taskmgr"),
            "Calendar" => ("win11-calendar", "/p/share/win11-gnome/icons/Win11/apps/scalable/office-calendar.svg", "Categories=Office;Calendar;\nMimeType=x-scheme-handler/win11-calendar;\n", " %u"),
            _ => ("win11-explorer", "system-file-manager", "Categories=System;FileTools;FileManager;\nMimeType=inode/directory;\n", " %U"),
        };
        format!(
            "[Desktop Entry]\nType=Application\nName={title}\nComment=Bestaande Win11Shell-interface in een zelfstandige Rust-host voor GNOME\nExec=/usr/bin/env -- \"/p with space/bin/{binary}\"{args}\nIcon={icon}\nTerminal=false\nStartupNotify=true\nStartupWMClass=io.github.win11shell.{name}\n{extra}"
        )
    }

    const NL_TITLES: [(&str, &str); 9] = [
        ("Panel", "Win11 Taakbalk"),
        ("Settings", "Win11 Instellingen"),
        ("DefaultApps", "Win11 Standaardapps"),
        ("Installer", "Win11 App-installatie"),
        ("Search", "Alles"),
        ("Disks", "Win11 Schijfbeheer"),
        ("TaskManager", "Win11 Taakbeheer"),
        ("Explorer", "Win11 Verkenner"),
        ("Calendar", "Agenda"),
    ];

    fn install_all(dir: &Path) {
        for (name, title) in NL_TITLES {
            fs::write(dir.join(format!("io.github.win11shell.{name}.desktop")), installed(name, title)).unwrap();
        }
    }

    #[test]
    fn the_table_has_the_nine_entries_with_install_py_dutch() {
        let table = table();
        assert_eq!(table.entries.len(), 9);
        for (name, title) in NL_TITLES {
            let entry = table.entries.iter().find(|e| e.name == name).unwrap();
            assert_eq!(entry.title[0], title, "{name} nl");
            assert!(!entry.title[1].is_empty(), "{name} en");
        }
        assert_eq!(table.comment[0], "Bestaande Win11Shell-interface in een zelfstandige Rust-host voor GNOME");
        assert_eq!(table.entries.iter().find(|e| e.name == "Search").unwrap().title[1], "Alles", "D-G5");
    }

    #[test]
    fn a_switch_rewrites_only_name_and_comment_and_switching_back_restores_the_bytes() {
        let dir = temp_dir();
        install_all(&dir);
        fs::set_permissions(dir.join("io.github.win11shell.Panel.desktop"), fs::Permissions::from_mode(0o640)).unwrap();
        let before: Vec<_> = NL_TITLES
            .iter()
            .map(|(n, _)| fs::read(dir.join(format!("io.github.win11shell.{n}.desktop"))).unwrap())
            .collect();
        let report = sync_dir(&dir, Lang::En);
        assert_eq!(report, Report { updated: 9, ..Default::default() });
        for ((name, _), old) in NL_TITLES.iter().zip(&before) {
            let path = dir.join(format!("io.github.win11shell.{name}.desktop"));
            let new = fs::read_to_string(&path).unwrap();
            let old = String::from_utf8(old.clone()).unwrap();
            let en = &table().entries.iter().find(|e| &e.name == name).unwrap().title[1];
            assert!(new.contains(&format!("\nName={en}\n")), "{new}");
            assert!(new.contains("\nComment=Existing Win11Shell interface in a standalone Rust host for GNOME\n"));
            let changed: Vec<_> = old.lines().zip(new.lines()).filter(|(a, b)| a != b).map(|(a, _)| a).collect();
            assert!(changed.iter().all(|l| l.starts_with("Name=") || l.starts_with("Comment=")), "{changed:?}");
            assert_eq!(old.lines().count(), new.lines().count());
        }
        let mode = fs::metadata(dir.join("io.github.win11shell.Panel.desktop")).unwrap().mode() & 0o777;
        assert_eq!(mode, 0o640, "mode kept");
        // the same language again: nothing is written (same inode, same mtime)
        let inode = fs::metadata(dir.join("io.github.win11shell.Explorer.desktop")).unwrap().ino();
        assert_eq!(sync_dir(&dir, Lang::En), Report { unchanged: 9, ..Default::default() });
        assert_eq!(fs::metadata(dir.join("io.github.win11shell.Explorer.desktop")).unwrap().ino(), inode);
        // back to Dutch: byte-identical to what install.py wrote
        assert_eq!(sync_dir(&dir, Lang::Nl).updated, 9);
        for ((name, _), old) in NL_TITLES.iter().zip(&before) {
            assert_eq!(&fs::read(dir.join(format!("io.github.win11shell.{name}.desktop"))).unwrap(), old, "{name}");
        }
        // no temporary file is left behind
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 9);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn foreign_missing_linked_and_other_groups_are_left_alone() {
        let dir = temp_dir();
        // not ours: another StartupWMClass under our file name
        let foreign = "[Desktop Entry]\nType=Application\nName=Iets\nComment=x\nStartupWMClass=org.example.Other\n";
        fs::write(dir.join("io.github.win11shell.Installer.desktop"), foreign).unwrap();
        // ours, with localized keys and an action group that also has Name=
        let ours = "[Desktop Entry]\r\nName=Win11 Verkenner\r\nName[nl]=Verkenner\r\nComment = oud\r\nStartupWMClass = io.github.win11shell.Explorer\r\nActions=new;\r\n\r\n[Desktop Action new]\r\nName=Nieuw venster\r\nComment=blijft\r\nStartupWMClass=io.github.win11shell.Explorer\r\n";
        fs::write(dir.join("io.github.win11shell.Explorer.desktop"), ours).unwrap();
        // a symlink to an entry elsewhere: never followed
        let target = dir.join("target.desktop");
        fs::write(&target, installed("Settings", "Win11 Instellingen")).unwrap();
        symlink(&target, dir.join("io.github.win11shell.Settings.desktop")).unwrap();
        // ours but without a Name line: skipped
        fs::write(dir.join("io.github.win11shell.Disks.desktop"), "[Desktop Entry]\nStartupWMClass=io.github.win11shell.Disks\n").unwrap();
        let report = sync_dir(&dir, Lang::En);
        assert_eq!(report.updated, 1);
        assert_eq!(report.skipped, 8);
        assert!(report.failed.is_empty());
        assert_eq!(fs::read_to_string(dir.join("io.github.win11shell.Installer.desktop")).unwrap(), foreign);
        assert_eq!(
            fs::read_to_string(dir.join("io.github.win11shell.Explorer.desktop")).unwrap(),
            "[Desktop Entry]\r\nName=Win11 File Explorer\r\nName[nl]=Verkenner\r\nComment=Existing Win11Shell interface in a standalone Rust host for GNOME\r\nStartupWMClass = io.github.win11shell.Explorer\r\nActions=new;\r\n\r\n[Desktop Action new]\r\nName=Nieuw venster\r\nComment=blijft\r\nStartupWMClass=io.github.win11shell.Explorer\r\n"
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), installed("Settings", "Win11 Instellingen"));
        assert!(!dir.join("io.github.win11shell.Panel.desktop").exists(), "nothing is created");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_scheduled_rewrite_uses_the_language_at_run_time_and_only_the_newest_one_runs() {
        use std::sync::{Arc, atomic::AtomicU8};
        let dir = temp_dir();
        install_all(&dir);
        // what the settings file says; the switch that scheduled the rewrite said Dutch, the file is English by the
        // time the worker runs (two quick switches dispatched out of order: review m5)
        let file = Arc::new(AtomicU8::new(Lang::Nl.as_u8()));
        let asked = Arc::new(AtomicU64::new(0));
        let source = |file: &Arc<AtomicU8>, asked: &Arc<AtomicU64>| {
            let (file, asked) = (file.clone(), asked.clone());
            move || {
                asked.fetch_add(1, Ordering::SeqCst);
                Lang::from_u8(file.load(Ordering::SeqCst))
            }
        };
        let older = {
            let dir = dir.clone();
            schedule_into(move || dir, Duration::from_millis(400), source(&file, &asked)).unwrap()
        };
        let newer = {
            let dir = dir.clone();
            schedule_into(move || dir, Duration::from_millis(100), source(&file, &asked)).unwrap()
        };
        file.store(Lang::En.as_u8(), Ordering::SeqCst);
        let report = newer.join().unwrap().expect("the newest call runs");
        assert_eq!(report.updated, 9, "written in the language of the moment, not the scheduled one");
        assert!(older.join().unwrap().is_none(), "a replaced call writes nothing");
        assert_eq!(asked.load(Ordering::SeqCst), 1, "the replaced call never asked for the language");
        let explorer = fs::read_to_string(dir.join("io.github.win11shell.Explorer.desktop")).unwrap();
        assert!(explorer.contains("\nName=Win11 File Explorer\n"), "{explorer}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn values_are_escaped_as_the_desktop_entry_spec_says() {
        assert_eq!(escape_value(" a\\b\nc\td"), "\\sa\\\\b\\nc\\td");
        assert_eq!(escape_value("Win11 Start"), "Win11 Start");
    }
}
