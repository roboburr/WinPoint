//! `background.*` (batch F, notes/claude/settings-parity.md §5.7): GNOME Settings 50's background chooser for
//! Instellingen › Persoonlijke instellingen › Achtergrond.
//!
//! * `background.catalog {}` → `{items, own, folder}`: the pictures of `gnome-background-properties/*.xml` in the
//!   system data dirs and then the user's (cc-background-xml.c's order): Ubuntu's and GNOME's pictures with their
//!   dark twins (`filename-dark`) and timed slideshows (a `.xml` file, with its first picture as `preview`), names in
//!   WinPoint's language; `own` are the user's pictures in `~/.local/share/backgrounds`, newest first
//!   (bg-recent-source.c).
//! * `background.add! {path}` → `{path, copied}`: GNOME's "Add Picture…": the picture copied into
//!   `~/.local/share/backgrounds` as `<YYYY-mm-dd-HH-MM-SS>-<name>` (bg_recent_source_add_file); a picture already
//!   in that folder is not copied again.
//! * `background.remove! {path}` → `{path}`: one of those copies moved to the trash (GNOME trashes it too); nothing
//!   outside the folder.
//!
//! Which picture is the wallpaper stays WinPoint's look (settings.json `wallpaper.*`, written to GNOME by
//! wallpaper.rs through gnome_look): the page sets `wallpaper.path` to a picture this service lists. Writes skip
//! under `ctx.dry_run` and emit `background.changed {what, path}`.
use super::ServiceContext;
use crate::files::{ApiError, ApiResult};
use crate::i18n::{Lang, NO_PARAMS};
use gio::prelude::*;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The methods (name, dangerous), exactly as host/src/api-methods/apps.json lists them (a test compares).
#[cfg_attr(not(test), allow(dead_code))] // the dispatch tests compare it with the api-methods files
pub(crate) const METHODS: &[(&str, bool)] = &[
    ("background.add", true),
    ("background.catalog", false),
    ("background.remove", true),
];
/// The largest picture `background.add` copies.
const MAX_PICTURE: u64 = 256 * 1024 * 1024;
/// The largest catalogue or slideshow file read.
const MAX_XML: u64 = 4 * 1024 * 1024;
/// At most this many of the user's own pictures are listed.
const MAX_OWN: usize = 200;

pub(crate) struct Background {
    ctx: Arc<ServiceContext>,
    /// The data dirs: the user's, then the system's (tests: a fixture tree).
    dirs: Dirs,
}

/// Where the catalogue and the user's pictures are.
#[derive(Clone, Debug)]
pub(crate) struct Dirs {
    pub user: PathBuf,
    pub system: Vec<PathBuf>,
}

impl Default for Dirs {
    fn default() -> Dirs {
        Dirs {
            user: glib::user_data_dir(),
            system: glib::system_data_dirs(),
        }
    }
}

impl Dirs {
    /// `~/.local/share/backgrounds`, the folder of the user's own pictures.
    pub(crate) fn own(&self) -> PathBuf {
        self.user.join("backgrounds")
    }
}

fn error(lang: Lang, key: &str) -> ApiError {
    let code = match key {
        "background.badArgs" | "background.notAnImage" | "background.tooLarge" => "bad_args",
        "background.notFound" => "not_found",
        "background.notOwn" => "denied",
        _ => "failed",
    };
    ApiError::tr(code, lang, key, NO_PARAMS)
}

// ------------------------------------------------------------------------------------------------ the catalogue

/// One picture of a gnome-background-properties file.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Item {
    pub name: String,
    pub light: PathBuf,
    pub dark: Option<PathBuf>,
    pub options: Option<String>,
    pub slideshow: bool,
    pub preview: Option<PathBuf>,
    pub source: String,
}

/// A name of the language WinPoint shows: nl (nl, nl_NL, ...), or en (en_GB, then en, then the untranslated one).
fn pick_name(names: &[(Option<String>, String)], language: &str) -> String {
    let untranslated = names
        .iter()
        .find(|(l, _)| l.is_none())
        .map(|(_, n)| n.clone());
    let lang_of = |want: &[&str]| {
        want.iter().find_map(|w| {
            names
                .iter()
                .find(|(l, _)| l.as_deref() == Some(*w))
                .map(|(_, n)| n.clone())
        })
    };
    let found = if language == "nl" {
        lang_of(&["nl", "nl_NL"]).or_else(|| {
            names
                .iter()
                .find(|(l, _)| l.as_deref().is_some_and(|l| l.starts_with("nl_")))
                .map(|(_, n)| n.clone())
        })
    } else {
        lang_of(&["en_GB", "en"])
    };
    found.or(untranslated).unwrap_or_default().trim().to_owned()
}

fn parse_options() -> roxmltree::ParsingOptions {
    roxmltree::ParsingOptions {
        allow_dtd: true,
        nodes_limit: 200_000,
    }
}

fn read_small(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_XML {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// The first picture of a GNOME slideshow (`<background>`'s first `<static><file>`, or its first `<size>`).
pub(crate) fn slideshow_preview(text: &str) -> Option<PathBuf> {
    let doc = roxmltree::Document::parse_with_options(text, parse_options()).ok()?;
    let root = doc.root_element();
    if root.tag_name().name() != "background" {
        return None;
    }
    for st in root.children().filter(|n| n.has_tag_name("static")) {
        let Some(file) = st.children().find(|n| n.has_tag_name("file")) else {
            continue;
        };
        let direct = file.text().map(str::trim).filter(|t| !t.is_empty());
        let sized = file
            .children()
            .find(|n| n.has_tag_name("size"))
            .and_then(|n| n.text())
            .map(str::trim);
        if let Some(path) = direct.or(sized).filter(|p| p.starts_with('/')) {
            return Some(PathBuf::from(path));
        }
    }
    None
}

/// The pictures of one gnome-background-properties file (`exists` checks a file, `slideshow_text` reads a
/// slideshow; tests use a fixture tree).
pub(crate) fn parse_catalog(
    text: &str,
    source: &str,
    language: &str,
    exists: &dyn Fn(&Path) -> bool,
    slideshow_text: &dyn Fn(&Path) -> Option<String>,
) -> Vec<Item> {
    let Ok(doc) = roxmltree::Document::parse_with_options(text, parse_options()) else {
        return Vec::new();
    };
    let root = doc.root_element();
    if root.tag_name().name() != "wallpapers" {
        return Vec::new();
    }
    let mut out = Vec::new();
    for wp in root.children().filter(|n| n.has_tag_name("wallpaper")) {
        if wp.attribute("deleted") == Some("true") {
            continue;
        }
        let mut names = Vec::new();
        let (mut light, mut dark, mut options) = (None, None, None);
        for child in wp.children().filter(|n| n.is_element()) {
            let text = child.text().map(str::trim).unwrap_or("").to_owned();
            match child.tag_name().name() {
                "name" => {
                    let lang = child.attribute(("http://www.w3.org/XML/1998/namespace", "lang"));
                    names.push((lang.map(str::to_owned), text));
                }
                "filename" => light = Some(text),
                "filename-dark" => dark = Some(text),
                "options" => options = Some(text),
                _ => {}
            }
        }
        // "(none)" was GNOME's solid colour; GNOME Settings 50 offers only pictures
        let Some(light) = light
            .filter(|f| f.starts_with('/'))
            .map(PathBuf::from)
            .filter(|f| exists(f))
        else {
            continue;
        };
        let dark = dark
            .filter(|f| f.starts_with('/'))
            .map(PathBuf::from)
            .filter(|f| exists(f) && *f != light);
        let slideshow = light.extension().is_some_and(|e| e == "xml");
        let preview = if slideshow {
            slideshow_text(&light)
                .and_then(|t| slideshow_preview(&t))
                .filter(|p| exists(p))
        } else {
            None
        };
        if slideshow && preview.is_none() {
            continue; // nothing to show it with
        }
        let mut name = pick_name(&names, language);
        if name.is_empty() {
            name = light
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
        out.push(Item {
            name,
            light,
            dark,
            options: options.filter(|o| !o.is_empty()),
            slideshow,
            preview,
            source: source.to_owned(),
        });
    }
    out
}

/// The `.xml` files of a gnome-background-properties folder, by name.
fn catalog_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "xml"))
        .collect();
    files.sort();
    files
}

/// The whole catalogue: the system data dirs first, then the user's, without doubles.
pub(crate) fn catalog(dirs: &Dirs, language: &str) -> Vec<Item> {
    let exists = |p: &Path| p.is_file();
    let slides = |p: &Path| read_small(p);
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for dir in dirs.system.iter().chain(std::iter::once(&dirs.user)) {
        for file in catalog_files(&dir.join("gnome-background-properties")) {
            let Some(text) = read_small(&file) else {
                continue;
            };
            let source = file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            for item in parse_catalog(&text, &source, language, &exists, &slides) {
                if seen.insert((item.light.clone(), item.dark.clone())) {
                    out.push(item);
                }
            }
        }
    }
    out
}

/// The user's own pictures: the regular files in `~/.local/share/backgrounds`, newest first.
pub(crate) fn own(dirs: &Dirs) -> Vec<(PathBuf, i64)> {
    let mut out: Vec<(PathBuf, i64)> = std::fs::read_dir(dirs.own())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| {
            let meta = std::fs::symlink_metadata(e.path()).ok()?;
            meta.is_file().then(|| {
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_millis() as i64);
                (e.path(), modified)
            })
        })
        .collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out.truncate(MAX_OWN);
    out
}

fn path_text(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

// ------------------------------------------------------------------------------------------------ add / remove

/// Whether GIO takes the file for a picture (its content type, from the name and the first bytes).
fn is_picture(path: &Path) -> bool {
    let mut head = [0u8; 4096];
    let n = std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut head))
        .unwrap_or(0);
    let (kind, _) = gio::content_type_guess(Some(path), &head[..n]);
    gio::content_type_get_mime_type(&kind).is_some_and(|m| m.starts_with("image/"))
}

/// The copy's name: GNOME's `<YYYY-mm-dd-HH-MM-SS>-<name>` (with `-2`, `-3`, ... when that name is taken).
pub(crate) fn copy_name(stamp: &str, name: &str, n: u32) -> String {
    if n <= 1 {
        format!("{stamp}-{name}")
    } else {
        format!("{stamp}-{n}-{name}")
    }
}

/// A file directly inside `folder` (no symlink, the folder itself resolved), or None.
fn inside(folder: &Path, path: &Path) -> Option<PathBuf> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let parent = path.parent()?.canonicalize().ok()?;
    let name = path.file_name()?;
    if name.to_string_lossy().starts_with('.') {
        return None;
    }
    (parent == folder.canonicalize().ok()?).then(|| parent.join(name))
}

impl Background {
    pub(crate) fn new(ctx: Arc<ServiceContext>) -> Result<Arc<Self>, String> {
        Ok(Arc::new(Self {
            ctx,
            dirs: Dirs::default(),
        }))
    }

    #[cfg(test)]
    pub(crate) fn with(ctx: Arc<ServiceContext>, dirs: Dirs) -> Arc<Self> {
        Arc::new(Self { ctx, dirs })
    }

    pub(crate) fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
        let lang = self.ctx.lang();
        Some(match method {
            "background.catalog" => Ok(self.catalog()),
            "background.add" => self.add(args, lang),
            "background.remove" => self.remove(args, lang),
            _ => return None,
        })
    }

    pub(crate) fn shutdown(&self) {}

    fn changed(&self, what: &str, path: &Path) {
        self.ctx.emit(
            "background.changed",
            json!({ "what": what, "path": path_text(path) }),
        );
    }

    fn catalog(&self) -> Value {
        let items: Vec<Value> = catalog(&self.dirs, &self.ctx.language())
            .into_iter()
            .map(|i| {
                json!({
                    "name": i.name, "path": path_text(&i.light), "dark": i.dark.as_deref().map(path_text),
                    "options": i.options, "slideshow": i.slideshow, "preview": i.preview.as_deref().map(path_text),
                    "source": i.source,
                })
            })
            .collect();
        let own: Vec<Value> = own(&self.dirs)
            .into_iter()
            .map(|(p, modified)| {
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                json!({ "path": path_text(&p), "name": name, "modified": modified })
            })
            .collect();
        json!({ "items": items, "own": own, "folder": path_text(&self.dirs.own()) })
    }

    fn path_arg(args: &Value, lang: Lang) -> Result<PathBuf, ApiError> {
        let raw = args["path"]
            .as_str()
            .filter(|p| p.starts_with('/') && p.len() <= 4096 && !p.contains('\0'))
            .ok_or_else(|| error(lang, "background.badArgs"))?;
        Ok(PathBuf::from(raw))
    }

    fn add(&self, args: &Value, lang: Lang) -> ApiResult {
        let source = Self::path_arg(args, lang)?;
        let meta = std::fs::metadata(&source).map_err(|_| error(lang, "background.notFound"))?;
        if !meta.is_file() {
            return Err(error(lang, "background.notFound"));
        }
        if meta.len() > MAX_PICTURE {
            return Err(error(lang, "background.tooLarge"));
        }
        if !is_picture(&source) {
            return Err(error(lang, "background.notAnImage"));
        }
        let folder = self.dirs.own();
        // one of the user's own pictures already: chosen as it is
        if let Some(path) = inside(&folder, &source) {
            return Ok(json!({ "path": path_text(&path), "copied": false }));
        }
        let name = source
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|n| !n.is_empty())
            .ok_or_else(|| error(lang, "background.badArgs"))?;
        let stamp = glib::DateTime::now_local()
            .and_then(|d| d.format("%Y-%m-%d-%H-%M-%S"))
            .map(|s| s.to_string())
            .map_err(|_| error(lang, "background.copyFailed"))?;
        let free = |n: u32| folder.join(copy_name(&stamp, &name, n));
        let mut n = 1;
        while n < 100 && std::fs::symlink_metadata(free(n)).is_ok() {
            n += 1;
        }
        let target = free(n);
        if self.ctx.dry_run {
            return Ok(json!({ "path": path_text(&target), "copied": false }));
        }
        std::fs::create_dir_all(&folder).map_err(|_| error(lang, "background.copyFailed"))?;
        let copied = (|| -> std::io::Result<()> {
            let from = std::fs::File::open(&source)?;
            let mut to = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)?;
            std::io::copy(&mut from.take(MAX_PICTURE), &mut to)?;
            to.flush()?;
            to.sync_all()
        })();
        if copied.is_err() {
            let _ = std::fs::remove_file(&target);
            return Err(error(lang, "background.copyFailed"));
        }
        self.changed("added", &target);
        Ok(json!({ "path": path_text(&target), "copied": true }))
    }

    fn remove(&self, args: &Value, lang: Lang) -> ApiResult {
        let path = Self::path_arg(args, lang)?;
        if std::fs::symlink_metadata(&path).is_err() {
            return Err(error(lang, "background.notFound"));
        }
        let path =
            inside(&self.dirs.own(), &path).ok_or_else(|| error(lang, "background.notOwn"))?;
        if self.ctx.dry_run {
            return Ok(json!({ "path": path_text(&path) }));
        }
        gio::File::for_path(&path)
            .trash(gio::Cancellable::NONE)
            .map_err(|_| error(lang, "background.removeFailed"))?;
        self.changed("removed", &path);
        Ok(json!({ "path": path_text(&path) }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A temporary tree that goes with the test.
    struct Temp(PathBuf);

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp(tag: &str) -> Temp {
        let root =
            std::env::temp_dir().join(format!("background-{tag}-{}", crate::events::nonce()));
        std::fs::create_dir_all(&root).unwrap();
        Temp(root)
    }

    const PNG: &[u8] = &[
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, b'I', b'H', b'D', b'R', 0, 0,
        0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1f, 0x15, 0xc4, 0x89,
    ];

    fn catalog_xml(dir: &Path) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE wallpapers SYSTEM "gnome-wp-list.dtd">
<wallpapers>
  <wallpaper deleted="false">
    <name>Ubuntu</name>
    <name xml:lang="nl">Ubuntu (nl)</name>
    <name xml:lang="en_GB">Ubuntu (GB)</name>
    <filename>{d}/light.png</filename>
    <filename-dark>{d}/dark.png</filename-dark>
    <options>zoom</options>
    <pcolor>#2c001e</pcolor>
  </wallpaper>
  <wallpaper>
    <name>Gone</name>
    <filename>{d}/missing.png</filename>
  </wallpaper>
  <wallpaper deleted="true">
    <name>Deleted</name>
    <filename>{d}/light.png</filename>
  </wallpaper>
  <wallpaper>
    <name>No colour</name>
    <filename>(none)</filename>
  </wallpaper>
  <wallpaper>
    <name>Light only</name>
    <filename>{d}/light.png</filename>
    <filename-dark>{d}/missing-dark.png</filename-dark>
  </wallpaper>
  <wallpaper>
    <name>Slides</name>
    <filename>{d}/slides.xml</filename>
    <options>zoom</options>
  </wallpaper>
</wallpapers>"#,
            d = dir.display()
        )
    }

    fn fixture(root: &Path) -> Dirs {
        let pics = root.join("pics");
        std::fs::create_dir_all(&pics).unwrap();
        for f in ["light.png", "dark.png", "one.png"] {
            std::fs::write(pics.join(f), PNG).unwrap();
        }
        std::fs::write(
            pics.join("slides.xml"),
            format!(
                "<background><starttime><year>2026</year></starttime><static><duration>60.0</duration><file>{}</file></static><transition><duration>5</duration><from>a</from><to>b</to></transition></background>",
                pics.join("one.png").display()
            ),
        )
        .unwrap();
        let system = root.join("system");
        let user = root.join("user");
        for d in [&system, &user] {
            std::fs::create_dir_all(d.join("gnome-background-properties")).unwrap();
        }
        std::fs::write(
            system.join("gnome-background-properties/ubuntu-wallpapers.xml"),
            catalog_xml(&pics),
        )
        .unwrap();
        // the user's catalogue names a picture the system already lists: once only
        std::fs::write(
            user.join("gnome-background-properties/mine.xml"),
            format!(
                "<wallpapers><wallpaper><name>Again</name><filename>{0}/light.png</filename><filename-dark>{0}/dark.png</filename-dark></wallpaper><wallpaper><name>Mine</name><filename>{0}/one.png</filename></wallpaper></wallpapers>",
                pics.display()
            ),
        )
        .unwrap();
        Dirs {
            user,
            system: vec![system],
        }
    }

    #[test]
    fn the_catalogue_is_gnome_settings_list() {
        let root = temp("catalog");
        let dirs = fixture(&root.0);
        let nl = catalog(&dirs, "nl");
        let names: Vec<&str> = nl.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(
            names,
            ["Ubuntu (nl)", "Light only", "Slides", "Mine"],
            "{nl:#?}"
        );
        let first = &nl[0];
        assert_eq!(
            first.dark.as_deref(),
            Some(root.0.join("pics/dark.png").as_path())
        );
        assert_eq!(first.options.as_deref(), Some("zoom"));
        assert_eq!(first.source, "ubuntu-wallpapers.xml");
        assert!(
            nl[1].dark.is_none(),
            "a dark twin that is not there is left out"
        );
        assert!(nl[2].slideshow);
        assert_eq!(
            nl[2].preview.as_deref(),
            Some(root.0.join("pics/one.png").as_path())
        );
        let en = catalog(&dirs, "en");
        assert_eq!(en[0].name, "Ubuntu (GB)");
        // no en_GB name: the untranslated one
        let names = [
            (None, "Plain".to_owned()),
            (Some("de".to_owned()), "Schlicht".to_owned()),
        ];
        assert_eq!(pick_name(&names, "en"), "Plain");
        assert_eq!(pick_name(&names, "nl"), "Plain");
        let names = [
            (None, "Plain".to_owned()),
            (Some("nl_BE".to_owned()), "Vlaams".to_owned()),
        ];
        assert_eq!(pick_name(&names, "nl"), "Vlaams");
        // broken input never panics and lists nothing
        for text in [
            "",
            "<wallpapers>",
            "<background/>",
            "<wallpapers><wallpaper/></wallpapers>",
        ] {
            assert!(
                parse_catalog(text, "x.xml", "nl", &|_| true, &|_| None).is_empty(),
                "{text}"
            );
        }
        assert_eq!(
            slideshow_preview(
                "<background><static><file><size width=\"1\" height=\"1\">/a.png</size></file></static></background>"
            ),
            Some(PathBuf::from("/a.png"))
        );
        assert_eq!(
            slideshow_preview(
                "<background><static><file>relative.png</file></static></background>"
            ),
            None
        );
    }

    fn context(dry_run: bool) -> (Arc<ServiceContext>, Arc<Mutex<Vec<Value>>>, Temp) {
        let root = temp("ctx");
        let events = crate::events::Events::new(&root.0.join("events")).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = seen.clone();
        events.on(
            "background.changed",
            Arc::new(move |data: &Value| record.lock().unwrap().push(data.clone())),
        );
        let ctx = ServiceContext::new(
            events,
            root.0.join("config"),
            root.0.join("cache"),
            root.0.clone(),
            "settings".into(),
            dry_run,
        )
        .unwrap();
        (ctx, seen, root)
    }

    #[test]
    fn add_copies_as_gnome_settings_and_remove_trashes_only_the_copies() {
        let root = temp("add");
        let dirs = fixture(&root.0);
        let picture = root.0.join("pics/one.png");
        // a dry run plans the copy and writes nothing
        let (ctx, seen, _c) = context(true);
        let dry = Background::with(ctx, dirs.clone());
        let planned = dry
            .call("background.add", &json!({ "path": picture }))
            .unwrap()
            .unwrap();
        let name = Path::new(planned["path"].as_str().unwrap())
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(
            name.ends_with("-one.png") && name.len() == "2026-11-16-10-11-12-one.png".len(),
            "{name}"
        );
        assert!(!dirs.own().exists(), "the dry run made no folder");
        assert!(seen.lock().unwrap().is_empty());
        // the copy
        let (ctx, seen, _c) = context(false);
        let svc = Background::with(ctx, dirs.clone());
        let done = svc
            .call("background.add", &json!({ "path": picture }))
            .unwrap()
            .unwrap();
        assert_eq!(done["copied"], true);
        let copy = PathBuf::from(done["path"].as_str().unwrap());
        assert_eq!(copy.parent(), Some(dirs.own().as_path()));
        assert_eq!(std::fs::read(&copy).unwrap(), PNG);
        assert_eq!(seen.lock().unwrap()[0]["what"], "added");
        // a second add in the same second gets a name of its own
        let again = svc
            .call("background.add", &json!({ "path": picture }))
            .unwrap()
            .unwrap();
        assert_ne!(again["path"], done["path"]);
        // choosing one of the copies copies nothing
        let same = svc
            .call("background.add", &json!({ "path": copy }))
            .unwrap()
            .unwrap();
        assert_eq!(same["copied"], false);
        assert_eq!(
            Path::new(same["path"].as_str().unwrap()).file_name(),
            copy.file_name()
        );
        // the catalogue lists the copies, newest first
        let listed = svc.call("background.catalog", &json!({})).unwrap().unwrap();
        assert_eq!(listed["own"].as_array().unwrap().len(), 2);
        assert_eq!(listed["folder"], json!(dirs.own()));
        // refusals
        let text = root.0.join("notes.txt");
        std::fs::write(&text, "hello").unwrap();
        for (args, code) in [
            (json!({ "path": text }), "bad_args"),
            (json!({ "path": root.0.join("nothing.png") }), "not_found"),
            (json!({ "path": "relative.png" }), "bad_args"),
            (json!({}), "bad_args"),
        ] {
            let e = svc.call("background.add", &args).unwrap().unwrap_err();
            assert_eq!(e.code, code, "{args}");
        }
        // remove: never a file outside the folder, nor through a symlink in it
        let link = dirs.own().join("link.png");
        if std::os::unix::fs::symlink(&picture, &link).is_ok() {
            let e = svc
                .call("background.remove", &json!({ "path": link }))
                .unwrap()
                .unwrap_err();
            assert_eq!(e.code, "denied");
            let _ = std::fs::remove_file(&link);
        }
        let e = svc
            .call("background.remove", &json!({ "path": picture }))
            .unwrap()
            .unwrap_err();
        assert_eq!(e.code, "denied");
        assert!(picture.exists());
        let (ctx, _, _c) = context(true);
        Background::with(ctx, dirs.clone())
            .call("background.remove", &json!({ "path": copy }))
            .unwrap()
            .unwrap();
        assert!(copy.exists(), "the dry run removed nothing");
        // the trash: in tests/rust-sandbox's private HOME only (a copy in the home's own data dir; the whole
        // sandbox environment, not keyfile GSettings alone, or a hand run would fill the user's own Trash)
        if crate::services::privacy::tests::sandbox().is_some() {
            let home = Dirs {
                user: glib::user_data_dir().join(format!("bg-test-{}", crate::events::nonce())),
                system: Vec::new(),
            };
            let svc = Background::with(svc.ctx.clone(), home.clone());
            let done = svc
                .call("background.add", &json!({ "path": picture }))
                .unwrap()
                .unwrap();
            let copy = PathBuf::from(done["path"].as_str().unwrap());
            svc.call("background.remove", &json!({ "path": copy }))
                .unwrap()
                .unwrap();
            assert!(!copy.exists());
            assert_eq!(seen.lock().unwrap().last().unwrap()["what"], "removed");
            let _ = std::fs::remove_dir_all(&home.user);
        }
    }

    #[test]
    fn errors_render_in_both_languages() {
        for (language, text) in [
            ("en", "This file isn’t a picture."),
            ("nl", "Dit bestand is geen afbeelding."),
        ] {
            let lc = crate::services::system::common::LangContext::new(language, "settings");
            let root = temp("lang");
            let file = root.0.join("a.txt");
            std::fs::write(&file, "x").unwrap();
            let svc = Background::with(lc.ctx.clone(), fixture(&root.0));
            let e = svc
                .call("background.add", &json!({ "path": file }))
                .unwrap()
                .unwrap_err();
            assert_eq!(
                (e.key.as_deref(), e.message.as_str()),
                (Some("background.notAnImage"), text)
            );
        }
        assert_eq!(
            copy_name("2026-11-16-10-11-12", "a.png", 1),
            "2026-11-16-10-11-12-a.png"
        );
        assert_eq!(
            copy_name("2026-11-16-10-11-12", "a.png", 3),
            "2026-11-16-10-11-12-3-a.png"
        );
    }
}
