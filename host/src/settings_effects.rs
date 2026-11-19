//! Native equivalents of the original settings theme listeners and managed GTK CSS.
//! GNOME app/serve hosts never install caption CSS at startup or when its setting changes.
use gio::prelude::*;
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::{OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};

const CAP_BEGIN: &str = ">>> win11shell caption-buttons >>>";
const CAP_END: &str = "<<< win11shell caption-buttons <<<";
const CAP_LINE: &str = "/* >>> win11shell caption-buttons >>> Windows 11-vensterknoppen, beheerd door Win11 Shell (uitzetten: Instellingen > Persoonlijke instellingen > Kleuren). Niet wijzigen. */";
const THEME_BEGIN: &str = ">>> win11shell theme >>>";
const THEME_END: &str = "<<< win11shell theme <<<";
const THEME_LINE: &str = "/* >>> win11shell theme >>> Windows 11-uiterlijk (Fluent) voor GTK 4-apps, beheerd door Win11 Shell (install.sh; ./uninstall.sh haalt het weg). Niet wijzigen. */";
const BUTTON_LAYOUT: &str = ":minimize,maximize,close";

pub(crate) struct Effects {
    pub root: PathBuf,
    pub config_home: PathBuf,
    pub layer: bool,
    /// The Windows 11 caption buttons in other GTK apps (theme.captionButtons) are the panel host's job in GNOME
    /// too (D-G65): Robin, 13-11, "ik zie geen windows 3 knoppen rechts boven bij alle apps".
    pub captions: bool,
    /// The settings mirror (settings_mirror.rs, notes/claude/settings-parity.md §6.4) runs in the panel host only.
    pub mirror: bool,
    pub dry_run: bool,
}
impl Effects {
    pub(crate) fn apply(&self, settings: &Value, changed: &[String]) {
        if self.dry_run {
            return;
        }
        if self.mirror {
            crate::settings_mirror::apply(settings, changed);
        }
        let theme = &settings["theme"];
        let relevant = changed.iter().any(|s| {
            matches!(
                s.as_str(),
                "theme" | "theme.mode" | "theme.syncSystem" | "theme.gtkFamily"
            )
        });
        if relevant
            && theme["syncSystem"] == true
            && matches!(theme["mode"].as_str(), Some("light" | "dark"))
        {
            let family = family(theme);
            let result = sync_system_theme(theme["mode"].as_str().unwrap(), family);
            if result["ok"] != true {
                eprintln!("GNOME-systeemthema niet volledig bijgewerkt: {result}");
            }
        }
        if let Some(family) = gtk4_target(settings, Some(changed)) {
            let result = sync_gtk4_block(family, &self.config_home);
            if result["ok"] != true {
                eprintln!("GTK 4-thema niet bijgewerkt: {result}");
            }
        }
        if (self.layer || self.captions)
            && changed
                .iter()
                .any(|s| matches!(s.as_str(), "theme" | "theme.captionButtons"))
        {
            let result = apply_captions(
                theme["captionButtons"] != false,
                &self.root,
                &self.config_home,
            );
            if result["ok"] != true {
                eprintln!("Vensterknoppen niet volledig bijgewerkt: {result}");
            }
        }
    }
    /// At the panel host's start: the caption buttons as the setting says (D-G65), and with theme.syncSystem
    /// WinPoint's GNOME look (colour scheme, GTK and icon theme, window buttons): a GNOME or Ubuntu session may have
    /// put GNOME's own back since (gnome_look.rs, WinPoint's own login session).
    pub(crate) fn captions_at_start(&self, settings: &Value) {
        if !self.captions || self.dry_run {
            return;
        }
        let theme = &settings["theme"];
        if theme["syncSystem"] == true
            && let Some(mode @ ("light" | "dark")) = theme["mode"].as_str()
        {
            let family = family(theme);
            let result = sync_system_theme(mode, family);
            if result["ok"] != true {
                eprintln!("GNOME-systeemthema niet volledig bijgewerkt: {result}");
            }
            if let Some(family) = gtk4_target(settings, None) {
                let result = sync_gtk4_block(family, &self.config_home);
                if result["ok"] != true {
                    eprintln!("GTK 4-thema niet bijgewerkt: {result}");
                }
            }
        }
        let result = apply_captions(
            settings["theme"]["captionButtons"] != false,
            &self.root,
            &self.config_home,
        );
        if result["ok"] != true {
            eprintln!("Vensterknoppen niet volledig bijgewerkt: {result}");
        }
    }
    // Original settings.start() runs only in a layer desktop session. All GNOME Rust
    // hosts replace the old --mode serve backend; starting one must not rewrite GTK CSS.
    pub(crate) fn startup(&self, settings: &Value) {
        if !self.layer || self.dry_run {
            return;
        }
        let theme = &settings["theme"];
        if theme["syncSystem"] != false {
            sync_button_layout();
            if family(theme) == "system" {
                sync_gtk4_block("system", &self.config_home);
            }
        }
        apply_captions(
            theme["captionButtons"] != false,
            &self.root,
            &self.config_home,
        );
    }
}
fn family(theme: &Value) -> &str {
    if theme["gtkFamily"] == "system" {
        "system"
    } else {
        "fluent"
    }
}
/// The GTK 4 block's family (amendment 46): "system", no Fluent block, while high contrast is on (libadwaita and
/// the GTK 4 apps then show their own high-contrast style), else the theme's.
fn gtk4_family(settings: &Value) -> &str {
    if settings["accessibility"]["highContrast"] == true {
        "system"
    } else {
        family(&settings["theme"])
    }
}
/// The family the GTK 4 block follows after the change of `changed` (None: the panel host's start), or None when it
/// stays as it is: with theme.syncSystem and a light or dark mode only, for a change of the theme family or of high
/// contrast (so a theme change while high contrast is on never puts the block back).
fn gtk4_target<'a>(settings: &'a Value, changed: Option<&[String]>) -> Option<&'a str> {
    let theme = &settings["theme"];
    if theme["syncSystem"] != true || !matches!(theme["mode"].as_str(), Some("light" | "dark")) {
        return None;
    }
    let touched = changed.is_none_or(|changed| {
        changed.iter().any(|s| {
            matches!(
                s.as_str(),
                "theme" | "theme.gtkFamily" | "accessibility" | "accessibility.highContrast"
            )
        })
    });
    touched.then(|| gtk4_family(settings))
}
fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(glib::home_dir)
}
fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|s| !s.is_empty() && Path::new(s).is_absolute())
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/share"))
}
fn data_dirs() -> Vec<PathBuf> {
    std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or("/usr/local/share:/usr/share".into())
        .split(':')
        .filter(|s| s.starts_with('/'))
        .map(PathBuf::from)
        .collect()
}
fn theme_dirs() -> Vec<PathBuf> {
    [home().join(".themes"), data_home().join("themes")]
        .into_iter()
        .chain(data_dirs().into_iter().map(|d| d.join("themes")))
        .collect()
}
fn gtk_installed(name: &str) -> bool {
    theme_dirs().iter().any(|d| {
        let p = d.join(name);
        p.is_dir() && (p.join("gtk-3.0").is_dir() || p.join("index.theme").is_file())
    })
}
fn icon_installed(name: &str) -> bool {
    [data_home().join("icons"), home().join(".icons")]
        .into_iter()
        .chain(data_dirs().into_iter().map(|d| d.join("icons")))
        .any(|d| d.join(name).join("index.theme").is_file())
}
fn yaru_accent(name: &str) -> Option<&str> {
    let name = name.strip_suffix("-dark").unwrap_or(name);
    if name == "Yaru" {
        return Some("");
    }
    let accent = name.strip_prefix("Yaru-")?;
    (!accent.is_empty() && accent.bytes().all(|c| c.is_ascii_lowercase())).then_some(accent)
}
fn stock(name: &str) -> bool {
    matches!(name, "Adwaita" | "Adwaita-dark") || yaru_accent(name).is_some()
}
pub(crate) fn stock_twin(current: Option<&str>, dark: bool) -> Option<String> {
    let current = current?;
    if matches!(current, "Adwaita" | "Adwaita-dark") {
        return Some(if dark { "Adwaita-dark" } else { "Adwaita" }.into());
    }
    let accent = yaru_accent(current)?;
    let twin = format!(
        "Yaru{}{}",
        if accent.is_empty() {
            String::new()
        } else {
            format!("-{accent}")
        },
        if dark { "-dark" } else { "" }
    );
    (twin == current || gtk_installed(&twin)).then_some(twin)
}
fn fluent(dark: bool) -> Option<String> {
    if dark {
        ["Fluent-round-Dark", "Fluent-Dark"]
    } else {
        ["Fluent-round-Light", "Fluent-Light"]
    }
    .into_iter()
    .find(|s| gtk_installed(s))
    .map(str::to_owned)
}
fn stock_default(dark: bool, accent: Option<&str>) -> String {
    let suffix = if dark { "-dark" } else { "" };
    if let Some(accent) = accent.filter(|s| !s.is_empty() && *s != "orange") {
        let name = format!("Yaru-{accent}{suffix}");
        if yaru_accent(&name).is_some() && gtk_installed(&name) {
            return name;
        }
    }
    let yaru = format!("Yaru{suffix}");
    if gtk_installed(&yaru) {
        yaru
    } else {
        format!("Adwaita{suffix}")
    }
}
pub(crate) fn theme_targets(
    mode: &str,
    current: Option<&str>,
    family: &str,
    accent: Option<&str>,
) -> Value {
    let dark = mode == "dark";
    let mut out = json!({"color-scheme":if dark{"prefer-dark"}else{"prefer-light"}});
    let icon = if dark { "Win11-dark" } else { "Win11" };
    if icon_installed(icon) {
        out["icon-theme"] = json!(icon);
    }
    if family != "system"
        && let Some(gtk) = fluent(dark)
    {
        out["gtk-theme"] = json!(gtk);
        return out;
    }
    if let Some(twin) = stock_twin(current, dark) {
        out["gtk-theme"] = json!(twin);
    } else if family == "system" && !current.is_some_and(stock) {
        out["gtk-theme"] = json!(stock_default(dark, accent));
    }
    out
}
fn schema(id: &str) -> Option<gio::Settings> {
    let source = gio::SettingsSchemaSource::default()?;
    let schema = source.lookup(id, true)?;
    Some(gio::Settings::new_full(
        &schema,
        None::<&gio::SettingsBackend>,
        None,
    ))
}
fn get(settings: Option<&gio::Settings>, key: &str) -> Option<String> {
    let s = settings?;
    s.settings_schema().filter(|schema| schema.has_key(key))?;
    Some(s.string(key).to_string())
}
fn set(settings: Option<&gio::Settings>, key: &str, value: &str) -> bool {
    let Some(s) = settings else { return false };
    if !s
        .settings_schema()
        .is_some_and(|schema| schema.has_key(key))
    {
        return false;
    }
    // WinPoint's own session: GNOME's value is kept for the other sessions (gnome_look.rs)
    crate::gnome_look::note(s, key, &value.to_variant());
    if s.string(key) == value {
        return true;
    }
    s.set_string(key, value).is_ok()
}
pub(crate) fn sync_button_layout() -> Value {
    let settings = schema("org.gnome.desktop.wm.preferences");
    if get(settings.as_ref(), "button-layout").as_deref() == Some(BUTTON_LAYOUT) {
        return json!({"ok":true,"changed":false});
    }
    let ok = set(settings.as_ref(), "button-layout", BUTTON_LAYOUT);
    json!({"ok":ok,"changed":ok})
}
pub(crate) fn sync_system_theme(mode: &str, family: &str) -> Value {
    let settings = schema("org.gnome.desktop.interface");
    let current = get(settings.as_ref(), "gtk-theme");
    let accent = get(settings.as_ref(), "accent-color")
        .filter(|s| !s.is_empty() && s.bytes().all(|c| c.is_ascii_lowercase()));
    let targets = theme_targets(mode, current.as_deref(), family, accent.as_deref());
    let mut ok = true;
    for (key, value) in targets.as_object().unwrap() {
        ok &= set(settings.as_ref(), key, value.as_str().unwrap());
    }
    sync_button_layout();
    gio::Settings::sync();
    json!({"ok":ok,"set":targets})
}
fn css_of(name: Option<String>) -> Option<PathBuf> {
    let name = name?;
    theme_dirs()
        .into_iter()
        .map(|d| d.join(&name).join("gtk-4.0/gtk.css"))
        .find(|p| p.is_file())
}
pub(crate) fn sync_gtk4_block(family: &str, config: &Path) -> Value {
    let path = config.join("gtk-4.0/gtk.css");
    let result = if family == "system" {
        gtk4_remove(&path)
    } else {
        let dark = css_of(fluent(true));
        let light = css_of(fluent(false));
        if dark.is_none() && light.is_none() {
            return json!({"ok":true,"action":"no-fluent"});
        }
        gtk4_add(&path, dark.as_deref(), light.as_deref())
    };
    match result {
        Ok(action) => json!({"ok":true,"action":action}),
        Err(_) => json!({"ok":false,"action":"error"}),
    }
}

fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_owned()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(p)
    }
}
fn uri(p: &Path) -> String {
    gio::File::for_path(absolute(p)).uri().to_string()
}
pub(crate) fn caption_block(css: &Path) -> Vec<u8> {
    format!(
        "{CAP_LINE}\n@import url(\"{}\");\n/* {CAP_END} */\n",
        uri(css)
    )
    .into_bytes()
}
pub(crate) fn theme_block(dark: Option<&Path>, light: Option<&Path>) -> Vec<u8> {
    let mut out = format!("{THEME_LINE}\n");
    if let (Some(dark), Some(light)) = (dark, light) {
        out.push_str(&format!("@media (prefers-color-scheme: dark) {{ @import url(\"{}\"); }}\n@media (prefers-color-scheme: light) {{ @import url(\"{}\"); }}\n",uri(dark),uri(light)));
    } else if let Some(css) = dark.or(light) {
        out.push_str(&format!("@import url(\"{}\");\n", uri(css)));
    }
    out.push_str(&format!("/* {THEME_END} */\n"));
    out.into_bytes()
}
fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|s| s == needle.as_bytes())
}
fn count(haystack: &[u8], needle: &str) -> usize {
    haystack
        .windows(needle.len())
        .filter(|s| *s == needle.as_bytes())
        .count()
}
// Python str.splitlines(keepends=True), without decoding/replacing invalid UTF-8 bytes.
fn lines(data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let (mut start, mut i) = (0, 0);
    while i < data.len() {
        let n = if data[i] == b'\r' {
            if data.get(i + 1) == Some(&b'\n') {
                2
            } else {
                1
            }
        } else if [b'\n', 11, 12, 28, 29, 30].contains(&data[i]) {
            1
        } else if data[i..].starts_with(&[0xc2, 0x85]) {
            2
        } else if data[i..].starts_with(&[0xe2, 0x80, 0xa8])
            || data[i..].starts_with(&[0xe2, 0x80, 0xa9])
        {
            3
        } else {
            0
        };
        if n > 0 {
            i += n;
            out.push(&data[start..i]);
            start = i
        } else {
            i += 1
        }
    }
    if start < data.len() {
        out.push(&data[start..]);
    }
    out
}
pub(crate) fn strip(data: &[u8], caption: bool) -> (Vec<u8>, bool) {
    let (begin, end) = if caption {
        (CAP_BEGIN, CAP_END)
    } else {
        (THEME_BEGIN, THEME_END)
    };
    let lines = lines(data);
    let (mut out, mut found, mut i) = (Vec::new(), false, 0);
    while i < lines.len() {
        if contains(lines[i], begin) {
            found = true;
            let mut j = i + 1;
            while j < lines.len() && !contains(lines[j], end) {
                j += 1
            }
            if j < lines.len() {
                i = j + 1
            } else {
                i += 1;
                if caption {
                    while i < lines.len()
                        && contains(lines[i], "@import")
                        && contains(lines[i], "caption-buttons-gtk")
                    {
                        i += 1
                    }
                }
            }
            continue;
        }
        if contains(lines[i], end) {
            found = true;
            i += 1;
            continue;
        }
        out.extend_from_slice(lines[i]);
        i += 1
    }
    (out, found)
}
fn whitespace(data: &[u8]) -> bool {
    String::from_utf8_lossy(data)
        .chars()
        .all(|c| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}'))
}
fn rstrip_newlines(data: &[u8]) -> &[u8] {
    let n = data.iter().rposition(|c| *c != b'\n').map_or(0, |i| i + 1);
    &data[..n]
}
fn lstrip_newlines(data: &[u8]) -> &[u8] {
    let n = data.iter().position(|c| *c != b'\n').unwrap_or(data.len());
    &data[n..]
}
fn os_error(e: &io::Error) -> String {
    e.raw_os_error()
        .and_then(|code| {
            let p = unsafe { libc::strerror(code) };
            (!p.is_null()).then(|| {
                unsafe { std::ffi::CStr::from_ptr(p) }
                    .to_string_lossy()
                    .into_owned()
            })
        })
        .unwrap_or_else(|| e.to_string())
}
fn backup(path: &Path) -> io::Result<Option<PathBuf>> {
    let mut name = path.as_os_str().to_owned();
    name.push(".win11shell-backup");
    let bak = PathBuf::from(name);
    if fs::symlink_metadata(&bak).is_ok() {
        return Ok(None);
    }
    if path.is_symlink() {
        std::os::unix::fs::symlink(fs::read_link(path)?, &bak)?;
    } else {
        fs::copy(path, &bak)?;
        let meta = path.metadata()?;
        let mut times = fs::FileTimes::new();
        if let Ok(t) = meta.accessed() {
            times = times.set_accessed(t)
        }
        if let Ok(t) = meta.modified() {
            times = times.set_modified(t)
        }
        let _ = OpenOptions::new()
            .write(true)
            .open(&bak)
            .and_then(|file| file.set_times(times));
    }
    Ok(Some(bak))
}
fn atomic(path: &Path, data: &[u8], mode: u32) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(
        ".{}.{}.win11shell.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        crate::events::nonce()
    ));
    let result = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}
pub(crate) fn apply_caption(path: &Path, css: &Path, enabled: bool) -> Value {
    let mut res = json!({"ok":true,"changed":false,"action":"unchanged","path":path});
    let link = path.is_symlink();
    if path.is_dir() {
        res["ok"] = json!(false);
        res["action"] = json!("skipped");
        res["error"] = json!("is a directory");
        return res;
    }
    let existed = path.exists() || link;
    let old = if !link && existed {
        match fs::read(path) {
            Ok(data) => data,
            Err(e) => {
                res["ok"] = json!(false);
                res["action"] = json!("skipped");
                res["error"] = json!(format!("unreadable: {}", os_error(&e)));
                return res;
            }
        }
    } else {
        Vec::new()
    };
    let (base, had) = if link {
        if !enabled {
            return res;
        }
        let prefix = fs::canonicalize(path)
            .ok()
            .filter(|p| p.is_file())
            .map(|p| format!("@import url(\"{}\");\n", uri(&p)).into_bytes())
            .unwrap_or_default();
        (prefix, false)
    } else {
        strip(&old, true)
    };
    let new = if enabled {
        if !css.is_file() {
            res["ok"] = json!(false);
            res["action"] = json!("skipped");
            res["error"] = json!(format!("missing {}", css.display()));
            return res;
        }
        let block = caption_block(css);
        if !link && count(&old, CAP_BEGIN) == 1 && old.windows(block.len()).any(|s| s == block) {
            return res;
        }
        let mut new = base;
        if !new.is_empty() && !new.ends_with(b"\n") {
            new.push(b'\n')
        }
        if !new.is_empty() && !new.ends_with(b"\n\n") {
            new.push(b'\n')
        }
        new.extend(block);
        new
    } else if !had {
        return res;
    } else if whitespace(&base) {
        Vec::new()
    } else {
        let mut new = rstrip_newlines(&base).to_vec();
        new.push(b'\n');
        new
    };
    if !link && new == old {
        return res;
    }
    let result = (|| -> io::Result<(String, Option<PathBuf>)> {
        let mode = if existed && !link {
            path.metadata()?.permissions().mode() & 0o777
        } else {
            0o644
        };
        let bak = if existed && !had && (!whitespace(&old) || link) {
            backup(path)?
        } else {
            None
        };
        let action = if !enabled && new.is_empty() {
            fs::remove_file(path)?;
            "deleted"
        } else {
            atomic(path, &new, mode)?;
            if existed && enabled {
                if had { "updated" } else { "added" }
            } else if enabled {
                "created"
            } else {
                "removed"
            }
        };
        Ok((action.into(), bak))
    })();
    match result {
        Ok((action, bak)) => {
            res["changed"] = json!(true);
            res["action"] = json!(action);
            if let Some(bak) = bak {
                res["backup"] = json!(bak);
            }
        }
        Err(e) => {
            res["ok"] = json!(false);
            res["action"] = json!("skipped");
            res["error"] = json!(format!("not written: {}", os_error(&e)));
        }
    }
    res
}
pub(crate) fn apply_captions(enabled: bool, root: &Path, config: &Path) -> Value {
    let files = [
        ("gtk-3.0/gtk.css", "config/gtk/caption-buttons-gtk3.css"),
        ("gtk-4.0/gtk.css", "config/gtk/caption-buttons-gtk4.css"),
    ]
    .into_iter()
    .map(|(path, css)| apply_caption(&config.join(path), &root.join(css), enabled))
    .collect::<Vec<_>>();
    json!({"ok":files.iter().all(|f|f["ok"]==true),"changed":files.iter().any(|f|f["changed"]==true),"files":files})
}
fn percent_decode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        if data[i] == b'%'
            && i + 2 < data.len()
            && let (Some(a), Some(b)) = (
                char::from(data[i + 1]).to_digit(16),
                char::from(data[i + 2]).to_digit(16),
            )
        {
            out.push((a * 16 + b) as u8);
            i += 3;
            continue;
        }
        out.push(data[i]);
        i += 1
    }
    out
}
fn foreign_fluent(data: &[u8]) -> Option<PathBuf> {
    let mut rest = data;
    while let Some(i) = rest.windows(7).position(|s| s == b"@import") {
        rest = &rest[i + 7..];
        let mut p = 0;
        while rest.get(p).is_some_and(u8::is_ascii_whitespace) {
            p += 1
        }
        if p == 0 || !rest[p..].starts_with(b"url(") {
            continue;
        }
        p += 4;
        while rest.get(p).is_some_and(u8::is_ascii_whitespace) {
            p += 1
        }
        if matches!(rest.get(p), Some(b'\'' | b'"')) {
            p += 1
        }
        let start = p;
        while p < rest.len() && !b"\"')".contains(&rest[p]) {
            p += 1
        }
        if p == start {
            continue;
        }
        let value = &rest[start..p];
        let value = value.strip_prefix(b"file://").unwrap_or(value);
        let path = PathBuf::from(std::ffi::OsString::from_vec(percent_decode(value)));
        if contains(path.as_os_str().as_bytes(), "Fluent")
            && contains(path.as_os_str().as_bytes(), "/gtk-4.0/")
            && path.is_file()
        {
            return Some(path);
        }
    }
    None
}
pub(crate) fn gtk4_add(
    path: &Path,
    dark: Option<&Path>,
    light: Option<&Path>,
) -> io::Result<String> {
    for css in [dark, light].into_iter().flatten() {
        if !css.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} ontbreekt", css.display()),
            ));
        }
    }
    if path.is_dir() {
        return Ok("skipped:directory".into());
    }
    let link = path.is_symlink();
    let target = if link {
        fs::canonicalize(path).ok()
    } else {
        None
    };
    if let Some(target) = target.as_ref().filter(|p| {
        contains(p.as_os_str().as_bytes(), "Fluent")
            && contains(p.as_os_str().as_bytes(), "/gtk-4.0/")
            && p.is_file()
    }) {
        return Ok(format!("foreign:{}", target.display()));
    }
    let existed = path.exists() || link;
    let old = if link {
        target
            .filter(|p| p.is_file())
            .map(|p| format!("@import url(\"{}\");\n", uri(&p)).into_bytes())
            .unwrap_or_default()
    } else if existed {
        match fs::read(path) {
            Ok(data) => data,
            Err(e) => return Ok(format!("skipped:unreadable {}", os_error(&e))),
        }
    } else {
        Vec::new()
    };
    let (rest, had) = strip(&old, false);
    if !had && let Some(foreign) = foreign_fluent(&rest) {
        return Ok(format!("foreign:{}", foreign.display()));
    }
    let block = theme_block(dark, light);
    if !link && count(&old, THEME_BEGIN) == 1 && old.starts_with(&block) {
        return Ok("unchanged".into());
    }
    let mut new = block;
    let rest = lstrip_newlines(&rest);
    if !rest.is_empty() {
        new.push(b'\n');
        new.extend(rest);
    }
    let mode = if existed && !link {
        path.metadata()?.permissions().mode() & 0o777
    } else {
        0o644
    };
    if existed && !had && (!whitespace(&old) || link) {
        backup(path)?;
    }
    atomic(path, &new, mode)?;
    Ok(if had {
        "updated"
    } else if existed {
        "added"
    } else {
        "created"
    }
    .into())
}
pub(crate) fn gtk4_remove(path: &Path) -> io::Result<String> {
    if !path.is_file() || path.is_symlink() {
        return Ok("unchanged".into());
    }
    let old = match fs::read(path) {
        Ok(data) => data,
        Err(e) => return Ok(format!("skipped:unreadable {}", os_error(&e))),
    };
    let (rest, had) = strip(&old, false);
    if !had {
        return Ok("unchanged".into());
    }
    let rest = lstrip_newlines(&rest);
    if whitespace(rest) {
        fs::remove_file(path)?;
        Ok("deleted".into())
    } else {
        atomic(path, rest, path.metadata()?.permissions().mode() & 0o777)?;
        Ok("removed".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn paths(list: &[&str]) -> Vec<String> {
        list.iter().map(|p| (*p).to_owned()).collect()
    }

    #[test]
    fn high_contrast_keeps_the_fluent_block_out_of_gtk4_at_the_start_and_after_a_theme_change() {
        let synced = |high: bool, family: &str| {
            json!({
                "theme": {"syncSystem": true, "mode": "dark", "gtkFamily": family},
                "accessibility": {"highContrast": high},
            })
        };
        // high contrast on: the start and a theme change both sync "system" (the block leaves)
        let on = synced(true, "fluent");
        assert_eq!(gtk4_target(&on, None), Some("system"));
        assert_eq!(gtk4_target(&on, Some(&paths(&["theme"]))), Some("system"));
        assert_eq!(
            gtk4_target(&on, Some(&paths(&["theme.gtkFamily"]))),
            Some("system")
        );
        assert_eq!(
            gtk4_target(&on, Some(&paths(&["accessibility.highContrast"]))),
            Some("system")
        );
        // off again: the theme's family, the block is back
        let off = synced(false, "fluent");
        assert_eq!(gtk4_target(&off, None), Some("fluent"));
        assert_eq!(
            gtk4_target(&off, Some(&paths(&["accessibility"]))),
            Some("fluent")
        );
        assert_eq!(gtk4_target(&synced(false, "system"), None), Some("system"));
        // other changes, or no GNOME look sync: the block is left as it is
        assert_eq!(
            gtk4_target(&off, Some(&paths(&["theme.mode", "taskbar"]))),
            None
        );
        let unsynced = json!({"theme": {"syncSystem": false, "mode": "dark"}, "accessibility": {"highContrast": true}});
        assert_eq!(gtk4_target(&unsynced, None), None);
        let auto = json!({"theme": {"syncSystem": true, "mode": "auto"}});
        assert_eq!(gtk4_target(&auto, None), None);
    }

    #[test]
    fn the_system_family_takes_the_fluent_block_out_and_keeps_the_rest() {
        let config = std::env::temp_dir().join(format!("effects-gtk4-{}", crate::events::nonce()));
        fs::create_dir_all(config.join("gtk-4.0")).unwrap();
        let css = config.join("gtk-4.0/gtk.css");
        let fluent = config.join("Fluent/gtk-4.0/gtk.css");
        fs::create_dir_all(fluent.parent().unwrap()).unwrap();
        fs::write(&fluent, "").unwrap();
        let mut text = theme_block(Some(&fluent), None);
        text.extend_from_slice(b"/* the user's own */\nwindow { padding: 1px; }\n");
        fs::write(&css, text).unwrap();
        let high = json!({"theme": {"syncSystem": true, "mode": "light"}, "accessibility": {"highContrast": true}});
        let result = sync_gtk4_block(gtk4_target(&high, None).unwrap(), &config);
        assert_eq!(result["ok"], true, "{result}");
        let left = fs::read_to_string(&css).unwrap();
        assert!(!left.contains(THEME_BEGIN), "{left}");
        assert!(left.contains("the user's own"), "{left}");
        let _ = fs::remove_dir_all(&config);
    }
}
