//! Native file opening, desktop launch/trust and terminals; argv never passes through a shell.
use super::{ApiError, ApiResult, Context, abs_in, directory, io_error, read};
use crate::i18n::Lang;
use gio::prelude::*;
use glib::translate::ToGlibPtr;
use serde_json::{Value, json};
use std::{
    cell::Cell,
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs::{self, File, OpenOptions},
    io::{self, Read},
    os::{
        fd::AsRawFd,
        unix::{
            ffi::{OsStrExt, OsStringExt},
            fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
            net::UnixListener,
        },
    },
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(super) fn call(method: &str, args: &Value, ctx: &Context) -> Option<ApiResult> {
    match method {
        "files.open" => Some(open(args, ctx)),
        "files.openWith" => Some(open_with(args, ctx)),
        "files.openTerminal" => Some(open_terminal(args, ctx)),
        "files.trustLauncher" => Some(trust_launcher(args, ctx)),
        _ => None,
    }
}
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(v) => *v,
        Value::Number(v) => v.as_f64() != Some(0.0),
        Value::String(v) => !v.is_empty(),
        Value::Array(v) => !v.is_empty(),
        Value::Object(v) => !v.is_empty(),
    }
}
/// `error` with its `data` (the UI's details: the app id, the launcher's own error text …).
fn with_data(mut error: ApiError, data: Value) -> ApiError {
    error.data = Some(data);
    error
}
fn missing_app(id: &str, lang: Lang) -> ApiError {
    with_data(
        ApiError::tr(
            "not_found",
            lang,
            "files_launch.appNotFound",
            crate::i18n::NO_PARAMS,
        ),
        json!({"id":id}),
    )
}
fn desktop_info(id: &str) -> Option<gio::DesktopAppInfo> {
    if id.contains('\0') {
        None
    } else {
        gio::DesktopAppInfo::new(id)
    }
}
fn open(args: &Value, ctx: &Context) -> ApiResult {
    let lang = ctx.lang();
    if args["path"]
        .as_str()
        .is_some_and(|path| path.starts_with("trash:"))
    {
        return launch_own("explorer", &json!({"path":"trash:///"}), lang);
    }
    let path = abs_in(&args["path"], lang)?;
    if !path.exists() {
        return Err(ApiError::tr(
            "not_found",
            lang,
            "files.itemGone",
            crate::i18n::NO_PARAMS,
        ));
    }
    launch_file(&path, ctx)
}
fn open_with(args: &Value, ctx: &Context) -> ApiResult {
    let lang = ctx.lang();
    let path = abs_in(&args["path"], lang)?;
    let app = args["appId"]
        .as_str()
        .filter(|app| !app.is_empty())
        .ok_or_else(|| {
            ApiError::tr(
                "bad_args",
                lang,
                "files_launch.noApp",
                crate::i18n::NO_PARAMS,
            )
        })?;
    if !path.exists() {
        return Err(ApiError::tr(
            "not_found",
            lang,
            "files.itemGone",
            crate::i18n::NO_PARAMS,
        ));
    }
    launch_app(app, &[path_uri(&path)], ctx)
}
fn path_uri(path: &Path) -> String {
    let mut result = String::from("file://");
    for byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/!$&'()*+,;=:@".contains(byte) {
            result.push(*byte as char);
        } else {
            use std::fmt::Write;
            let _ = write!(result, "%{byte:02X}");
        }
    }
    result
}
fn launch_file(path: &Path, ctx: &Context) -> ApiResult {
    let lang = ctx.lang();
    if path.is_dir() {
        return launch_own("explorer", &json!({"path":path}), lang);
    }
    if path.as_os_str().as_bytes().ends_with(b".desktop") && path.is_file() {
        return launch_desktop_file(path, &[], ctx);
    }
    if crate::backend::dry_run() {
        return Ok(super::dry::result(json!({})));
    }
    let handler = gio::File::for_path(path)
        .query_default_handler(gio::Cancellable::NONE)
        .ok();
    launch_gio(
        json!({"action":"default","uri":path_uri(path)}),
        handler.as_ref(),
        lang,
    )
    .map_err(|error| {
        with_data(
            ApiError::tr(
                "no_app",
                lang,
                "files_launch.noAppForFile",
                crate::i18n::NO_PARAMS,
            ),
            json!(error),
        )
    })?;
    Ok(json!({}))
}
fn own_id(id: &str) -> Option<&'static str> {
    // own_app normalises to identify an alias, but callers only accept the exact canonical id.
    let folded = id.trim().to_lowercase();
    let folded = folded.strip_suffix(".desktop").unwrap_or(&folded);
    let folded = folded.strip_prefix("win11shell-").unwrap_or(folded);
    crate::launch::OWN_APPS
        .iter()
        .copied()
        .find(|app| *app == folded)
}
fn canonical_own(id: &str) -> Option<&'static str> {
    let app = own_id(id)?;
    (id.strip_suffix(".desktop").unwrap_or(id) == format!("win11shell-{app}")).then_some(app)
}
fn own_params(app: &str, uris: &[String]) -> Value {
    if app == "explorer" {
        return crate::app_params::explorer(uris);
    }
    let Some(uri) = uris.first() else {
        return json!({});
    };
    let Some(path) = gio::File::for_uri(uri).path() else {
        return json!({});
    };
    match app {
        "explorer" if path.is_dir() => json!({"path":path}),
        "explorer" => json!({"path":path.parent().unwrap_or(Path::new("/")),"select":[path]}),
        "alles" => json!({"query":path.file_name().unwrap_or_default().to_string_lossy()}),
        _ => json!({}),
    }
}
fn launch_own(app: &str, params: &Value, lang: Lang) -> ApiResult {
    if crate::backend::dry_run() {
        return Ok(super::dry::result(json!({})));
    }
    let mut command = crate::launch::command(app, Some(params))
        .map_err(|message| ApiError::new("launch_failed", &message))?;
    command.env_clear().envs(user_environment(false));
    crate::launch::prepare_activation(&mut command, app);
    spawn(command).map_err(|error| {
        with_data(
            ApiError::tr(
                "launch_failed",
                lang,
                "files_launch.appFailed",
                crate::i18n::NO_PARAMS,
            ),
            json!(error.to_string()),
        )
    })?;
    Ok(json!({}))
}
pub(crate) fn launch_app(app: &str, uris: &[String], ctx: &Context) -> ApiResult {
    let lang = ctx.lang();
    if app.contains('\0') {
        return Err(missing_app(app, lang));
    }
    if let Some(own) = canonical_own(app) {
        return launch_own(own, &own_params(own, uris), lang);
    }
    if app.starts_with('/') && !in_application_dirs(Path::new(app)) {
        return launch_desktop_file(Path::new(app), uris, ctx);
    }
    launch_installed(app, uris, None, lang)
}
fn launch_installed(
    app: &str,
    uris: &[String],
    info: Option<gio::DesktopAppInfo>,
    lang: Lang,
) -> ApiResult {
    if crate::backend::dry_run() {
        return Ok(super::dry::result(json!({})));
    }
    let info = info
        .or_else(|| {
            if app.starts_with('/') {
                gio::DesktopAppInfo::from_filename(app)
            } else {
                desktop_info(app)
            }
        })
        .ok_or_else(|| missing_app(app, lang))?;
    launch_gio(
        json!({"action":"desktop","filename":info.filename(),"id":info.id().map(|id|id.to_string()),"uris":uris}),
        Some(info.upcast_ref()),
        lang,
    )
    .map_err(|error| {
        with_data(
            ApiError::tr("launch_failed", lang, "files_launch.appFailed", crate::i18n::NO_PARAMS),
            json!(error),
        )
    })?;
    Ok(json!({}))
}
struct DesktopEntry {
    kind: String,
    name: String,
    exec: String,
    url: String,
}
thread_local! { static LINK_DEPTH:Cell<u8>=const{Cell::new(0)}; }
struct LinkDepth;
impl LinkDepth {
    fn enter(lang: Lang) -> Result<Self, ApiError> {
        LINK_DEPTH.with(|depth| {
            if depth.get() >= 32 {
                return Err(ApiError::tr(
                    "bad_entry",
                    lang,
                    "files_launch.cannotOpenShortcut",
                    crate::i18n::NO_PARAMS,
                ));
            }
            depth.set(depth.get() + 1);
            Ok(Self)
        })
    }
}
impl Drop for LinkDepth {
    fn drop(&mut self) {
        LINK_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}
fn desktop_entry(path: &Path) -> Option<DesktopEntry> {
    let keyfile = glib::KeyFile::new();
    keyfile
        .load_from_file(path, glib::KeyFileFlags::KEEP_TRANSLATIONS)
        .ok()?;
    if !keyfile.has_group("Desktop Entry") {
        return None;
    }
    let text = |key| {
        keyfile
            .string("Desktop Entry", key)
            .map(|v| v.to_string())
            .unwrap_or_default()
    };
    let kind = text("Type");
    let mut name = keyfile
        .locale_string("Desktop Entry", "Name", None)
        .map(|v| v.to_string())
        .unwrap_or_default();
    if name.is_empty() {
        name = text("Name");
    }
    Some(DesktopEntry {
        kind: if kind.is_empty() {
            "Application".into()
        } else {
            kind
        },
        name,
        exec: text("Exec"),
        url: text("URL"),
    })
}
fn application_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![glib::user_data_dir().join("applications")];
    let env = std::env::var_os("XDG_DATA_DIRS")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    dirs.extend(
        std::env::split_paths(&env)
            .filter(|path| path.is_absolute())
            .map(|path| path.join("applications")),
    );
    dirs.extend([
        PathBuf::from("/var/lib/flatpak/exports/share/applications"),
        PathBuf::from("/var/lib/snapd/desktop/applications"),
        glib::user_data_dir().join("flatpak/exports/share/applications"),
    ]);
    dirs.into_iter()
        .map(|dir| fs::canonicalize(&dir).unwrap_or(dir))
        .collect()
}
fn in_application_dirs(path: &Path) -> bool {
    let real = fs::canonicalize(path).unwrap_or_else(|_| path.into());
    application_dirs().iter().any(|dir| real.starts_with(dir))
}
const NO_PERMISSION_FS: &[&str] = &[
    "vfat",
    "msdos",
    "exfat",
    "ntfs",
    "ntfs3",
    "fuseblk",
    "fuse.ntfs-3g",
    "fuse.exfat",
    "udf",
    "iso9660",
    "hfsplus",
    "fuse.sshfs",
    "cifs",
    "smb3",
    "9p",
];
fn unsafe_filesystem(path: &Path) -> bool {
    let real = fs::canonicalize(path).unwrap_or_else(|_| path.into());
    NO_PERMISSION_FS.contains(&read::filesystem_type(&real).as_str())
}
fn launcher_trust(path: &Path) -> (bool, &'static str) {
    let Ok(metadata) = fs::metadata(path) else {
        return (false, "missing");
    };
    if !metadata.is_file() {
        return (false, "not_file");
    }
    if unsafe_filesystem(path) {
        return (false, "filesystem");
    }
    if metadata.uid() != unsafe { libc::getuid() } || metadata.mode() & 0o100 == 0 {
        return (false, "not_executable");
    }
    let marked = gio::File::for_path(path)
        .query_info(
            "metadata::trusted",
            gio::FileQueryInfoFlags::NONE,
            gio::Cancellable::NONE,
        )
        .ok()
        .and_then(|info| info.attribute_as_string("metadata::trusted"))
        .is_some_and(|value| matches!(value.trim().to_lowercase().as_str(), "true" | "yes" | "1"));
    if !marked {
        return (false, "not_marked");
    }
    (true, "")
}
fn untrusted(path: &Path, entry: &DesktopEntry, reason: &str, lang: Lang) -> ApiError {
    let basename = path.file_name().unwrap_or_default().to_string_lossy();
    let error = if reason == "filesystem" {
        ApiError::tr(
            "untrusted",
            lang,
            "files_launch.untrustedDrive",
            crate::i18n::NO_PARAMS,
        )
    } else {
        ApiError::tr(
            "untrusted",
            lang,
            "files_launch.untrusted",
            crate::i18n::NO_PARAMS,
        )
    };
    with_data(
        error,
        json!({"path":path,"name":if entry.name.is_empty(){basename.as_ref()}else{&entry.name},"exec":entry.exec.chars().take(500).collect::<String>(),"reason":reason,"canTrust":reason!="filesystem"}),
    )
}
fn launch_desktop_file(path: &Path, uris: &[String], ctx: &Context) -> ApiResult {
    let lang = ctx.lang();
    let base = path.file_name().unwrap_or_default().to_string_lossy();
    if base.ends_with(".desktop") {
        if let Some(own) = canonical_own(&base) {
            return launch_own(own, &own_params(own, uris), lang);
        }
    }
    if !base.ends_with(".desktop") || !path.is_file() {
        return Err(missing_app(path.to_string_lossy().as_ref(), lang));
    }
    let entry = desktop_entry(path).ok_or_else(|| {
        ApiError::tr(
            "bad_entry",
            lang,
            "files_launch.damagedShortcut",
            crate::i18n::NO_PARAMS,
        )
    })?;
    if entry.kind == "Link" {
        let _depth = LinkDepth::enter(lang)?;
        return open_uri(&entry.url, ctx);
    }
    if entry.kind != "Application" {
        return Err(ApiError::tr(
            "bad_entry",
            lang,
            "files_launch.cannotOpenShortcut",
            crate::i18n::NO_PARAMS,
        ));
    }
    let id = path.to_string_lossy();
    if in_application_dirs(path) {
        return launch_installed(&id, uris, None, lang);
    }
    let (trusted, reason) = launcher_trust(path);
    if !trusted {
        if let Some(installed) = desktop_info(&base) {
            return launch_installed(&base, uris, Some(installed), lang);
        }
        return Err(untrusted(path, &entry, reason, lang));
    }
    launch_installed(&id, uris, gio::DesktopAppInfo::from_filename(path), lang)
}
pub(crate) fn open_uri(uri: &str, ctx: &Context) -> ApiResult {
    let lang = ctx.lang();
    let stripped = uri.trim();
    if uri.is_empty() || uri.chars().count() > 8192 || stripped.chars().any(|c| c < ' ' || c == ' ')
    {
        return Err(ApiError::tr(
            "bad_uri",
            lang,
            "files_launch.cannotOpenLink",
            crate::i18n::NO_PARAMS,
        ));
    }
    let scheme = stripped
        .split_once(':')
        .map(|(scheme, _)| scheme.to_lowercase())
        .unwrap_or_default();
    if !matches!(scheme.as_str(), "http" | "https" | "mailto" | "file") {
        return Err(with_data(
            ApiError::tr(
                "bad_uri",
                lang,
                "files_launch.unsupportedLink",
                crate::i18n::NO_PARAMS,
            ),
            json!({"scheme":scheme}),
        ));
    }
    if matches!(scheme.as_str(), "http" | "https") {
        let authority = stripped
            .split_once(':')
            .unwrap()
            .1
            .strip_prefix("//")
            .unwrap_or("")
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("");
        if authority.is_empty() {
            return Err(ApiError::tr(
                "bad_uri",
                lang,
                "files_launch.invalidLink",
                crate::i18n::NO_PARAMS,
            ));
        }
        // Match urlsplit's rejected IPv6 delimiters before handing the URI to GIO.
        if (authority.contains('[') || authority.contains(']'))
            && url::Url::parse(stripped).is_err()
        {
            return Err(ApiError::tr(
                "bad_uri",
                lang,
                "files_launch.invalidLink",
                crate::i18n::NO_PARAMS,
            ));
        }
        if authority.chars().filter(|ch| !ch.is_ascii()).any(|ch| {
            glib::normalize(ch.to_string(), glib::NormalizeMode::AllCompose)
                .chars()
                .any(|c| "/?#@:".contains(c))
        }) {
            return Err(ApiError::tr(
                "bad_uri",
                lang,
                "files_launch.invalidLink",
                crate::i18n::NO_PARAMS,
            ));
        }
    }
    if scheme == "file" {
        let path = local_file_uri(stripped)
            .filter(|path| path.exists())
            .ok_or_else(|| {
                ApiError::tr(
                    "not_found",
                    lang,
                    "files_launch.fileGone",
                    crate::i18n::NO_PARAMS,
                )
            })?;
        return launch_file(&path, ctx);
    }
    if crate::backend::dry_run() {
        return Ok(super::dry::result(json!({})));
    }
    let handler = gio::AppInfo::default_for_uri_scheme(&scheme);
    launch_gio(
        json!({"action":"default","uri":stripped}),
        handler.as_ref(),
        lang,
    )
    .map_err(|error| {
        with_data(
            ApiError::tr(
                "no_app",
                lang,
                "files_launch.noAppForLink",
                crate::i18n::NO_PARAMS,
            ),
            json!(error),
        )
    })?;
    Ok(json!({}))
}
fn local_file_uri(uri: &str) -> Option<PathBuf> {
    let (_, raw) = uri.split_once(':')?;
    let raw = raw.strip_prefix("//").unwrap_or(raw);
    let raw = raw.split(['?', '#']).next()?;
    let (host, path) = raw.split_once('/')?;
    if !matches!(host, "" | "localhost") {
        return None;
    }
    Some(PathBuf::from(super::percent_decode(&format!("/{path}"))))
}
fn trust_launcher(args: &Value, ctx: &Context) -> ApiResult {
    let lang = ctx.lang();
    let path = abs_in(&args["path"], lang)?;
    if !path.as_os_str().as_bytes().ends_with(b".desktop") || !path.is_file() {
        return Err(ApiError::tr(
            "bad_args",
            lang,
            "files_launch.notShortcut",
            crate::i18n::NO_PARAMS,
        ));
    }
    if unsafe_filesystem(&path) {
        return Err(ApiError::tr(
            "filesystem",
            lang,
            "files_launch.untrustedDriveTrust",
            crate::i18n::NO_PARAMS,
        ));
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_CLOEXEC)
        .open(&path)
        .map_err(|error| io_error(error, &path, lang))?;
    let metadata = file
        .metadata()
        .map_err(|error| io_error(error, &path, lang))?;
    if !metadata.is_file() {
        return Err(ApiError::tr(
            "bad_args",
            lang,
            "files_launch.notShortcut",
            crate::i18n::NO_PARAMS,
        ));
    }
    if metadata.uid() != unsafe { libc::getuid() } {
        return Err(ApiError::tr(
            "no_access",
            lang,
            "files_launch.foreignShortcut",
            crate::i18n::NO_PARAMS,
        ));
    }
    gio::File::for_path(&path)
        .set_attribute_string(
            "metadata::trusted",
            "true",
            gio::FileQueryInfoFlags::NONE,
            gio::Cancellable::NONE,
        )
        .map_err(|error| {
            // PyGObject's str(GError) contains the domain and numeric code, unlike message.
            let pointer: *const glib::ffi::GError = error.to_glib_none().0;
            let message = format!(
                "{}: {} ({})",
                error.domain().as_str(),
                error.message(),
                unsafe { (*pointer).code }
            );
            with_data(
                ApiError::tr(
                    "failed",
                    lang,
                    "files_launch.trustFailed",
                    crate::i18n::NO_PARAMS,
                ),
                json!(message.chars().take(200).collect::<String>()),
            )
        })?;
    let current = fs::metadata(&path).map_err(|error| io_error(error, &path, lang))?;
    if current.dev() != metadata.dev() || current.ino() != metadata.ino() {
        return Err(io_error(
            io::Error::from_raw_os_error(libc::EBUSY),
            &path,
            lang,
        ));
    }
    set_owner_execute(&file, metadata.mode()).map_err(|error| io_error(error, &path, lang))?;
    ctx.changed(path.parent().unwrap_or(Path::new("/")));
    Ok(json!({"ok":true,"trusted":launcher_trust(&path).0}))
}
fn set_owner_execute(file: &File, mode: u32) -> io::Result<()> {
    // O_PATH supports a launcher without read permission. This names the held inode.
    fs::set_permissions(
        format!("/proc/self/fd/{}", file.as_raw_fd()),
        fs::Permissions::from_mode((mode | 0o100) & 0o7777),
    )
}

const TERMINALS: &[&str] = &[
    "org.gnome.Ptyxis.desktop",
    "org.gnome.Terminal.desktop",
    "org.gnome.Console.desktop",
    "kitty.desktop",
    "com.mitchellh.ghostty.desktop",
    "Alacritty.desktop",
    "foot.desktop",
    "org.wezfurlong.wezterm.desktop",
    "org.kde.konsole.desktop",
    "xfce4-terminal.desktop",
    "terminator.desktop",
    "tilix.desktop",
    "com.gexperts.Tilix.desktop",
    "xterm.desktop",
    "debian-xterm.desktop",
];
fn terminal_lists(config: PathBuf, dirs: &OsStr, desktops: &str) -> Vec<PathBuf> {
    let mut configs = vec![config];
    configs.extend(std::env::split_paths(dirs).filter(|path| !path.as_os_str().is_empty()));
    let desktops: Vec<_> = desktops
        .split(':')
        .filter(|part| !part.is_empty())
        .map(str::to_lowercase)
        .collect();
    configs
        .into_iter()
        .flat_map(|directory| {
            let mut result: Vec<_> = desktops
                .iter()
                .map(|desktop| directory.join(format!("{desktop}-xdg-terminals.list")))
                .collect();
            result.push(directory.join("xdg-terminals.list"));
            result
        })
        .collect()
}
fn terminal_ids(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with(['#', '-']))
        .map(|line| line.trim_start_matches('+').split(':').next().unwrap_or(""))
}
pub(crate) fn default_terminal() -> Option<gio::DesktopAppInfo> {
    let dirs = std::env::var_os("XDG_CONFIG_DIRS")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/etc/xdg".into());
    let desktops = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    for list in terminal_lists(glib::user_config_dir(), &dirs, &desktops) {
        if let Ok(text) = fs::read_to_string(list) {
            for id in terminal_ids(&text) {
                if let Some(info) = desktop_info(id) {
                    return Some(info);
                }
            }
        }
    }
    let terminals: Vec<_> = gio::AppInfo::all()
        .into_iter()
        .filter_map(|info| info.downcast::<gio::DesktopAppInfo>().ok())
        .filter(|info| {
            info.categories()
                .is_some_and(|categories| categories.split(';').any(|c| c == "TerminalEmulator"))
        })
        .collect();
    for id in TERMINALS {
        if let Some(info) = terminals
            .iter()
            .find(|info| info.id().as_deref() == Some(*id))
        {
            return Some(info.clone());
        }
    }
    for id in TERMINALS {
        if let Some(info) = desktop_info(id) {
            return Some(info);
        }
    }
    terminals.into_iter().next()
}
fn terminal_command(executable: PathBuf, directory: &Path) -> Command {
    let mut command = Command::new(&executable);
    let base = executable.file_name().unwrap_or_default().to_string_lossy();
    match base.as_ref() {
        "gnome-terminal" | "kgx" | "foot" | "ghostty" | "xfce4-terminal" | "tilix" => {
            command.arg(format!("--working-directory={}", directory.display()));
        }
        "ptyxis" => {
            command.arg("--new-window").arg("-d").arg(directory);
        }
        "kitty" => {
            command.arg("--directory").arg(directory);
        }
        "alacritty" => {
            command.arg("--working-directory").arg(directory);
        }
        "konsole" => {
            command.arg("--workdir").arg(directory);
        }
        "wezterm" => {
            command.arg("start").arg("--cwd").arg(directory);
        }
        _ => {}
    }
    command.current_dir(directory);
    command
}
fn open_terminal(args: &Value, ctx: &Context) -> ApiResult {
    let lang = ctx.lang();
    let home = json!(ctx.home);
    let input = if truthy(&args["dir"]) {
        &args["dir"]
    } else if truthy(&args["path"]) {
        &args["path"]
    } else {
        &home
    };
    let directory = directory(input, lang)?;
    let info = default_terminal().ok_or_else(|| {
        ApiError::tr(
            "no_terminal",
            lang,
            "files_launch.noTerminal",
            crate::i18n::NO_PARAMS,
        )
    })?;
    let executable = info.executable();
    let executable = if !executable.as_os_str().is_empty() && !executable.is_absolute() {
        glib::find_program_in_path(&executable)
    } else {
        Some(executable)
    }
    .filter(|path| !path.as_os_str().is_empty())
    .ok_or_else(|| {
        ApiError::tr(
            "no_terminal",
            lang,
            "files_launch.terminalNotFound",
            crate::i18n::NO_PARAMS,
        )
    })?;
    let mut command = terminal_command(executable, &directory);
    command.env_clear().envs(user_environment(true));
    command
        .env_remove("DESKTOP_STARTUP_ID")
        .env_remove("XDG_ACTIVATION_TOKEN");
    if let Some(token) = info
        .id()
        .and_then(|id| crate::launch::activation_token(&json!({"desktopId":id.as_str()})))
    {
        command
            .env("DESKTOP_STARTUP_ID", &token)
            .env("XDG_ACTIVATION_TOKEN", token);
    }
    spawn(command).map_err(|_| {
        ApiError::tr(
            "launch_failed",
            lang,
            "files_launch.terminalFailed",
            crate::i18n::NO_PARAMS,
        )
    })?;
    Ok(json!({}))
}
fn spawn(command: Command) -> io::Result<()> {
    if crate::backend::dry_run() {
        return Ok(());
    }
    // Both callers constructed a complete, sanitized environment with env_clear.
    let env = command
        .get_envs()
        .filter_map(|(key, value)| value.map(|value| (key.into(), value.into())))
        .collect();
    crate::process_scope::spawn(command, "file-open", Some(env)).map_err(io::Error::other)
}

/// Starts `request` through the standalone win11-launch helper and waits for its answer; the error is the
/// helper's own text or one of ours in `lang` (it goes into ApiError.data).
fn launch_gio(mut request: Value, info: Option<&gio::AppInfo>, lang: Lang) -> Result<(), String> {
    if crate::backend::dry_run() {
        return Ok(());
    }
    let executable = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .with_file_name("win11-launch");
    if !executable.is_file() {
        return Err(crate::i18n::tr(lang, "files_launch.launcherMissing"));
    }
    let context = launch_context_for(info);
    let env: Vec<(OsString, OsString)> = context
        .environment()
        .into_iter()
        .filter_map(|entry| {
            let bytes = entry.as_bytes();
            let split = bytes.iter().position(|byte| *byte == b'=')?;
            Some((
                OsString::from_vec(bytes[..split].to_vec()),
                OsString::from_vec(bytes[split + 1..].to_vec()),
            ))
        })
        .collect();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let directory =
        std::env::temp_dir().join(format!("win11-launch-{}-{nonce}", std::process::id()));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .map_err(|e| e.to_string())?;
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_file(self.0.join("ack"));
            let _ = fs::remove_dir(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let socket = directory.join("ack");
    let listener = UnixListener::bind(&socket).map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    request["ack"] = json!(socket);
    let mut command = Command::new(executable);
    command
        .arg(request.to_string())
        .env_clear()
        .envs(env.clone());
    crate::process_scope::spawn(command, "external", Some(env))?;
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .map_err(|e| e.to_string())?;
                let mut text = String::new();
                stream
                    .take(65536)
                    .read_to_string(&mut text)
                    .map_err(|e| e.to_string())?;
                let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                return if value["ok"] == true {
                    Ok(())
                } else {
                    Err(value["error"].as_str().map_or_else(
                        || crate::i18n::tr(lang, "files_launch.launchFailed"),
                        str::to_owned,
                    ))
                };
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(crate::i18n::tr(lang, "files_launch.launcherNoReply"));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

type Environment = BTreeMap<OsString, OsString>;
/// The GTK switches a WinPoint process may set for itself: GSK_RENDERER, which win11-shell, win11-calendar and the
/// native apps set to cairo when the user chose none, and GDK_DEBUG, which the native apps set to `portals` (their
/// file dialog is the desktop's FileChooser portal; native/kit app.rs). What a launched app gets instead is the
/// WIN11SHELL_USER_GPU marker's values (the user's own: written by those programs, and by [`environment`] for our
/// own children), else the `win11shell` supervisor's, else this process's own.
const GPU_KEYS: &[&str] = &["GSK_RENDERER", "GDK_DEBUG"];
fn gpu_values(env: &Environment, supervisor: Option<Environment>) -> Environment {
    if let Some(raw) = env.get(OsStr::new("WIN11SHELL_USER_GPU")) {
        if let Ok(Value::Object(values)) = serde_json::from_slice::<Value>(raw.as_bytes()) {
            return values
                .into_iter()
                .filter(|(key, _)| GPU_KEYS.contains(&key.as_str()))
                .filter_map(|(key, value)| {
                    value
                        .as_str()
                        .map(|value| (OsString::from(key), OsString::from(value)))
                })
                .collect();
        }
    }
    supervisor
        .unwrap_or_else(|| env.clone())
        .into_iter()
        .filter(|(key, _)| GPU_KEYS.iter().any(|gpu| key == OsStr::new(gpu)))
        .collect()
}
fn environment(
    env: &Environment,
    supervisor: Option<Environment>,
    drop_private: bool,
) -> Environment {
    let values = gpu_values(env, supervisor);
    let mut result = env.clone();
    for key in GPU_KEYS {
        result.remove(OsStr::new(key));
    }
    result.extend(values.clone());
    if drop_private {
        result.retain(|key, _| {
            let key = key.to_string_lossy();
            !key.starts_with("WIN11SHELL_")
                && !key.starts_with("WIN11_DEV_")
                && !key.starts_with("WIN11_TEST_")
                && !matches!(key.as_ref(), "WIN11_REFERENCE_INSTANCE" | "WIN11_UI_DIR")
        });
    } else {
        let values: BTreeMap<_, _> = values
            .into_iter()
            .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)))
            .collect();
        result.insert(
            "WIN11SHELL_USER_GPU".into(),
            serde_json::to_string(&values).unwrap().into(),
        );
    }
    result
}
fn supervisor_environment() -> Option<Environment> {
    let parent = unsafe { libc::getppid() };
    if parent <= 1 {
        return None;
    }
    if fs::metadata(format!("/proc/{parent}")).ok()?.uid() != unsafe { libc::getuid() } {
        return None;
    }
    let cmdline = fs::read(format!("/proc/{parent}/cmdline")).ok()?;
    let args: Vec<_> = cmdline
        .split(|byte| *byte == 0)
        .filter(|bytes| !bytes.is_empty())
        .map(OsStr::from_bytes)
        .collect();
    if !matches!(
        Path::new(args.first()?).file_name().and_then(OsStr::to_str),
        Some("bash" | "sh")
    ) || !args
        .iter()
        .skip(1)
        .take(2)
        .any(|arg| Path::new(arg).file_name() == Some(OsStr::new("win11shell")))
    {
        return None;
    }
    let bytes = fs::read(format!("/proc/{parent}/environ")).ok()?;
    Some(
        bytes
            .split(|byte| *byte == 0)
            .filter_map(|entry| {
                let split = entry.iter().position(|byte| *byte == b'=')?;
                if split == 0 {
                    return None;
                }
                Some((
                    OsString::from_vec(entry[..split].to_vec()),
                    OsString::from_vec(entry[split + 1..].to_vec()),
                ))
            })
            .collect(),
    )
}
pub(crate) fn user_environment(drop_private: bool) -> Environment {
    environment(
        &std::env::vars_os().collect(),
        supervisor_environment(),
        drop_private,
    )
}
/// Restart an existing application with its own environment, restoring only this
/// host's renderer overrides and excluding private shell launch variables.
pub(crate) fn restart_environment(
    mut base: BTreeMap<OsString, OsString>,
) -> BTreeMap<OsString, OsString> {
    let clean = user_environment(true);
    base.retain(|key, _| {
        !key.to_string_lossy().starts_with("WIN11SHELL_")
            && !key.to_string_lossy().starts_with("WIN11_GNOME_")
            && !GPU_KEYS.iter().any(|gpu| key == OsStr::new(gpu))
    });
    for key in GPU_KEYS {
        if let Some(value) = clean.get(OsStr::new(key)) {
            base.insert(OsString::from(key), value.clone());
        }
    }
    base
}
fn launch_context_for(info: Option<&gio::AppInfo>) -> gio::AppLaunchContext {
    let token = info
        .and_then(|info| info.id())
        .and_then(|id| crate::launch::activation_token(&json!({"desktopId":id.as_str()})));
    let context = crate::launch_context::new(token.clone());
    let current: Environment = std::env::vars_os().collect();
    let wanted = environment(&current, supervisor_environment(), true);
    for key in current.keys() {
        if !wanted.contains_key(key) {
            context.unsetenv(key);
        }
    }
    for (key, value) in wanted {
        if current.get(&key) != Some(&value) {
            context.setenv(key, value);
        }
    }
    crate::launch_context::configure_portal_context(&context);
    // Keep environment-based activation as well as the context virtual method:
    // newly spawned processes and existing D-Bus applications use different paths.
    context.unsetenv("DESKTOP_STARTUP_ID");
    context.unsetenv("XDG_ACTIVATION_TOKEN");
    if let Some(token) = token {
        context.setenv("DESKTOP_STARTUP_ID", &token);
        context.setenv("XDG_ACTIVATION_TOKEN", &token);
    }
    context
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::unix::fs::symlink,
        sync::atomic::{AtomicU64, Ordering},
    };
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static ID: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "win11-actions-{}-{}",
                std::process::id(),
                ID.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn context(&self) -> Context {
            Context::new(self.0.clone(), self.0.join("Desktop"), "nl".into(), false)
        }
        fn launcher(&self, contents: &str) -> PathBuf {
            let path = self.0.join("win11-action-unit-uninstalled-unique.desktop");
            fs::write(&path, contents).unwrap();
            path
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn env(pairs: &[(&str, &str)]) -> Environment {
        pairs
            .iter()
            .map(|(k, v)| (OsString::from(k), OsString::from(v)))
            .collect()
    }
    fn args(command: &Command) -> Vec<String> {
        command
            .get_args()
            .map(|v| v.to_string_lossy().into_owned())
            .collect()
    }
    #[test]
    fn malformed_uri_authority_is_rejected_before_any_handler_launch() {
        let ctx = Context::new(
            PathBuf::from("/tmp"),
            PathBuf::from("/tmp"),
            "en".into(),
            false,
        );
        for uri in ["http://[", "http://]", "https://example.invalid：80"] {
            let error = open_uri(uri, &ctx).unwrap_err();
            assert_eq!(error.code, "bad_uri");
            assert_eq!(error.message, "This link isn’t valid.");
            assert_eq!(error.key.as_deref(), Some("files_launch.invalidLink"));
        }
    }
    #[test]
    fn gpu_user_marker_restores_exact_values_and_strips_foreign_private_env() {
        let base = env(&[
            (
                "WIN11SHELL_USER_GPU",
                r#"{"GSK_RENDERER":"cairo","GTK_THEME":"ignored","unrelated":"ignored"}"#,
            ),
            ("GSK_RENDERER", "vulkan"),
            // only GSK_RENDERER is ours: the marker's other keys are ignored, an app keeps its own value
            ("GTK_THEME", "user"),
            ("WIN11SHELL_TOKEN", "secret"),
            ("WIN11SHELL_TRASH_DIR", "private"),
            ("WIN11SHELL_GPU_ENV", r#"{"set":["GSK_RENDERER"]}"#),
            ("WIN11_DEV_COMMAND", "private"),
            ("WIN11_REFERENCE_INSTANCE", "private"),
            ("WIN11_UI_DIR", "private"),
            ("WIN11_TEST_APP", "private"),
            ("DISPLAY", ":123"),
            ("PATH", "/user/bin"),
        ]);
        let clean = environment(&base, None, true);
        assert_eq!(
            clean.get(OsStr::new("GSK_RENDERER")),
            Some(&OsString::from("cairo"))
        );
        assert_eq!(
            clean.get(OsStr::new("GTK_THEME")),
            Some(&OsString::from("user"))
        );
        assert!(
            clean
                .keys()
                .all(|key| !key.to_string_lossy().starts_with("WIN11"))
        );
        assert_eq!(
            clean.get(OsStr::new("DISPLAY")),
            Some(&OsString::from(":123"))
        );
        let own = environment(&base, None, false);
        assert_eq!(
            own.get(OsStr::new("WIN11SHELL_TOKEN")),
            base.get(OsStr::new("WIN11SHELL_TOKEN"))
        );
        let marker: Value =
            serde_json::from_slice(own[OsStr::new("WIN11SHELL_USER_GPU")].as_bytes()).unwrap();
        assert_eq!(marker, json!({"GSK_RENDERER":"cairo"}));
    }
    #[test]
    fn the_supervisor_and_empty_or_broken_user_markers_decide_the_renderer() {
        let base = env(&[("GSK_RENDERER", "cairo")]);
        let supervisor = env(&[("GSK_RENDERER", "opengl"), ("PATH", "/supervisor/bin")]);
        assert_eq!(
            environment(&base, Some(supervisor), true),
            env(&[("GSK_RENDERER", "opengl")]),
            "the supervisor's renderer, nothing else of it"
        );
        assert!(environment(&base, Some(Environment::new()), true).is_empty());
        let base = env(&[("WIN11SHELL_USER_GPU", "{}"), ("GSK_RENDERER", "vulkan")]);
        assert!(environment(&base, None, true).is_empty());
        // a native app's backend (native/kit app.rs): its cairo and portals are its own, the user's GDK_DEBUG stays
        let base = env(&[
            ("WIN11SHELL_USER_GPU", r#"{"GDK_DEBUG":"misc"}"#),
            ("GSK_RENDERER", "cairo"),
            ("GDK_DEBUG", "portals"),
        ]);
        assert_eq!(
            environment(&base, None, true),
            env(&[("GDK_DEBUG", "misc")])
        );
        let base = env(&[
            ("WIN11SHELL_USER_GPU", "malformed"),
            ("GSK_RENDERER", "vulkan"),
        ]);
        assert_eq!(
            environment(&base, None, true),
            env(&[("GSK_RENDERER", "vulkan")])
        );
    }
    #[test]
    fn terminal_argv_is_literal_for_every_supported_terminal() {
        let path = Path::new("/tmp/folder with spaces/'quote'\n$(touch forbidden)`no`");
        for name in [
            "gnome-terminal",
            "ptyxis",
            "kgx",
            "kitty",
            "foot",
            "alacritty",
            "ghostty",
            "konsole",
            "xfce4-terminal",
            "wezterm",
            "tilix",
            "unknown-terminal",
        ] {
            let command = terminal_command(PathBuf::from(format!("/private/bin/{name}")), path);
            let argv = args(&command);
            assert_eq!(command.get_current_dir(), Some(path));
            assert_eq!(
                command.get_program(),
                OsStr::new(&format!("/private/bin/{name}"))
            );
            let expected: Vec<String> = match name {
                "ptyxis" => vec![
                    "--new-window".into(),
                    "-d".into(),
                    path.to_string_lossy().into_owned(),
                ],
                "kitty" => vec!["--directory".into(), path.to_string_lossy().into_owned()],
                "alacritty" => vec![
                    "--working-directory".into(),
                    path.to_string_lossy().into_owned(),
                ],
                "konsole" => vec!["--workdir".into(), path.to_string_lossy().into_owned()],
                "wezterm" => vec![
                    "start".into(),
                    "--cwd".into(),
                    path.to_string_lossy().into_owned(),
                ],
                "unknown-terminal" => vec![],
                _ => vec![format!("--working-directory={}", path.display())],
            };
            assert_eq!(argv, expected, "{name}");
        }
    }
    #[test]
    fn terminal_list_precedence_and_entry_syntax_match_reference() {
        let lists = terminal_lists(
            PathBuf::from("/private/config"),
            OsStr::new("/etc/one:/etc/two"),
            "GNOME:Unity",
        );
        assert_eq!(
            lists,
            vec![
                "/private/config/gnome-xdg-terminals.list",
                "/private/config/unity-xdg-terminals.list",
                "/private/config/xdg-terminals.list",
                "/etc/one/gnome-xdg-terminals.list",
                "/etc/one/unity-xdg-terminals.list",
                "/etc/one/xdg-terminals.list",
                "/etc/two/gnome-xdg-terminals.list",
                "/etc/two/unity-xdg-terminals.list",
                "/etc/two/xdg-terminals.list"
            ]
            .into_iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>()
        );
        assert_eq!(
            terminal_ids(
                "# comment\n-missing.desktop\n +chosen.desktop:argument\n\n fallback.desktop\n"
            )
            .collect::<Vec<_>>(),
            ["chosen.desktop", "fallback.desktop"]
        );
    }
    #[test]
    fn file_uris_preserve_reference_percent_encoding_without_shell_expansion() {
        assert_eq!(
            path_uri(Path::new("/tmp/map !'$&()+,;=:@~é/#?%\"\n")),
            "file:///tmp/map%20!'$&()+,;=:@~%C3%A9/%23%3F%25%22%0A"
        );
        assert_eq!(
            local_file_uri("file://localhost/tmp/a%20b?query#fragment"),
            Some(PathBuf::from("/tmp/a b"))
        );
        assert!(local_file_uri("file://remote.example/tmp/file").is_none());
        assert!(local_file_uri("https://example.com/file").is_none());
    }
    #[test]
    fn own_app_ids_and_single_file_params_preserve_existing_host_route() {
        let temp = Temp::new();
        let path = temp.0.join("a ' $(no).txt");
        fs::write(&path, b"file").unwrap();
        assert_eq!(
            canonical_own("win11shell-explorer.desktop"),
            Some("explorer")
        );
        assert_eq!(canonical_own("win11shell-settings"), Some("settings"));
        for id in [
            "explorer",
            "EXPLORER",
            "WIN11SHELL-explorer.desktop",
            " win11shell-explorer.desktop ",
            "/tmp/win11shell-explorer.desktop",
        ] {
            assert_eq!(canonical_own(id), None);
        }
        assert_eq!(
            own_params("explorer", &[path_uri(&path)]),
            json!({"path":temp.0,"select":[path]})
        );
        assert_eq!(
            own_params("explorer", &[path_uri(&temp.0)]),
            json!({"path":temp.0})
        );
        assert_eq!(
            own_params("alles", &[path_uri(&path)]),
            json!({"query":"a ' $(no).txt"})
        );
        assert_eq!(own_params("settings", &[path_uri(&path)]), json!({}));
    }
    #[test]
    fn malformed_entries_and_untrusted_exec_return_before_any_launch() {
        let temp = Temp::new();
        let ctx = temp.context();
        let path = temp.launcher("invalid desktop file");
        assert_eq!(
            launch_desktop_file(&path, &[], &ctx).unwrap_err().code,
            "bad_entry"
        );
        let path = temp.launcher("[Desktop Entry]\nType=Directory\nName=Unsupported\n");
        assert_eq!(
            launch_desktop_file(&path, &[], &ctx).unwrap_err().message,
            "Deze snelkoppeling kan niet worden geopend."
        );
        let marker = temp.0.join("must-not-exist");
        let path=temp.launcher(&format!("[Desktop Entry]\nType=Application\nName=Untrusted fixture\nExec=/bin/sh -c 'touch {}'\n",marker.display()));
        let error = launch_desktop_file(&path, &[], &ctx).unwrap_err();
        assert_eq!(error.code, "untrusted");
        let data = error.data.unwrap();
        assert_eq!(data["reason"], "not_executable");
        assert_eq!(data["canTrust"], true);
        assert!(data["exec"].as_str().unwrap().contains("touch"));
        assert!(!marker.exists());
        let mut entry = desktop_entry(&path).unwrap();
        entry.exec = "🦀".repeat(501);
        assert_eq!(
            untrusted(&path, &entry, "not_marked", Lang::Nl)
                .data
                .unwrap()["exec"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            500
        );
        assert_eq!(launcher_trust(&temp.0), (false, "not_file"));
        assert_eq!(launcher_trust(&temp.0.join("missing")), (false, "missing"));
    }
    // round2-gate §6: the files_launch stem rendered through the real call sites (open, openWith, the launcher
    // checks, links, trustLauncher) for an English context, with key and params (C2); Dutch stays 3b898e6's.
    #[test]
    fn launch_errors_render_in_the_context_language_with_their_keys() {
        let temp = Temp::new();
        let en = Context::new(temp.0.clone(), temp.0.join("Desktop"), "en".into(), false);
        let nl = temp.context();
        let regular = temp.0.join("ordinary.txt");
        fs::write(&regular, b"file").unwrap();
        let no_app = open_with(&json!({"path":regular,"appId":""}), &en).unwrap_err();
        assert_eq!(no_app.message, "Choose an app.");
        assert_eq!(no_app.json()["key"], "files_launch.noApp");
        assert_eq!(
            open(&json!({"path":temp.0.join("missing")}), &en)
                .unwrap_err()
                .message,
            "This item no longer exists."
        );
        let path = temp.launcher("[Desktop Entry]\nType=Directory\nName=Unsupported\n");
        assert_eq!(
            launch_desktop_file(&path, &[], &en).unwrap_err().message,
            "This shortcut can’t be opened."
        );
        let path = temp.launcher("not a key file");
        assert_eq!(
            launch_desktop_file(&path, &[], &en).unwrap_err().message,
            "This shortcut is damaged."
        );
        let path =
            temp.launcher("[Desktop Entry]\nType=Application\nName=Fixture\nExec=/bin/true\n");
        let untrusted = launch_desktop_file(&path, &[], &en).unwrap_err();
        assert_eq!(untrusted.code, "untrusted");
        assert_eq!(
            untrusted.message,
            "This shortcut isn’t trusted yet, so it was blocked for your security. If you trust its source, \
             choose “Allow launching”."
        );
        assert_eq!(untrusted.json()["key"], "files_launch.untrusted");
        assert_eq!(untrusted.data.unwrap()["canTrust"], true);
        let dutch = launch_desktop_file(&path, &[], &nl).unwrap_err().message;
        assert!(dutch.ends_with("Kies dan ‘Starten toestaan’."), "{dutch}");
        let scheme = open_uri("javascript:alert(1)", &en).unwrap_err();
        assert_eq!(scheme.message, "This kind of link can’t be opened.");
        assert_eq!(scheme.data.unwrap(), json!({"scheme":"javascript"}));
        assert_eq!(
            open_uri("", &en).unwrap_err().message,
            "This link can’t be opened."
        );
        assert_eq!(
            open_uri("file:///nonexistent/win11-unit", &en)
                .unwrap_err()
                .message,
            "This file no longer exists."
        );
        assert_eq!(
            trust_launcher(&json!({"path":regular}), &en)
                .unwrap_err()
                .message,
            "This isn’t a shortcut."
        );
        assert_eq!(
            trust_launcher(&json!({"path":regular}), &nl)
                .unwrap_err()
                .message,
            "Dit is geen snelkoppeling."
        );
        let missing = missing_app("gone.desktop", Lang::En);
        assert_eq!(
            missing.message,
            "This app wasn’t found. It might have been uninstalled."
        );
        assert_eq!(missing.data.unwrap(), json!({"id":"gone.desktop"}));
    }
    #[test]
    fn invalid_file_actions_and_link_urls_fail_without_external_actions() {
        let temp = Temp::new();
        let ctx = temp.context();
        assert_eq!(
            open(&json!({"path":"relative"}), &ctx).unwrap_err().code,
            "bad_args"
        );
        assert_eq!(
            open(&json!({"path":temp.0.join("missing")}), &ctx)
                .unwrap_err()
                .code,
            "not_found"
        );
        assert_eq!(
            open_with(&json!({"path":temp.0.join("missing"),"appId":""}), &ctx)
                .unwrap_err()
                .message,
            "Kies een app."
        );
        let path =
            temp.launcher("[Desktop Entry]\nType=Link\nName=Unsafe\nURL=javascript:malicious\n");
        let error = launch_desktop_file(&path, &[], &ctx).unwrap_err();
        assert_eq!(error.code, "bad_uri");
        assert_eq!(error.data.unwrap(), json!({"scheme":"javascript"}));
        for uri in [
            "",
            "https:example.com",
            "https:///missing-host",
            "https://host/a b",
            "https://host/a\nB",
        ] {
            assert_eq!(open_uri(uri, &ctx).unwrap_err().code, "bad_uri");
        }
        assert_eq!(
            open_uri("file://elsewhere/tmp/file", &ctx)
                .unwrap_err()
                .code,
            "not_found"
        );
        let regular = temp.0.join("ordinary.txt");
        fs::write(&regular, b"file").unwrap();
        assert_eq!(
            open_with(
                &json!({"path":regular,"appId":"/usr/share/applications/bad\0.desktop"}),
                &ctx
            )
            .unwrap_err()
            .code,
            "not_found"
        );
        assert_eq!(
            trust_launcher(&json!({"path":regular}), &ctx)
                .unwrap_err()
                .code,
            "bad_args"
        );
        let broken = temp.0.join("broken.desktop");
        symlink("missing", &broken).unwrap();
        assert_eq!(
            trust_launcher(&json!({"path":broken}), &ctx)
                .unwrap_err()
                .code,
            "bad_args"
        );
    }

    #[test]
    fn allowing_execute_requires_no_read_access_and_pins_the_original_file() {
        let temp = Temp::new();
        let path = temp.0.join("no-read.desktop");
        fs::write(
            &path,
            b"[Desktop Entry]\nType=Application\nExec=/bin/false\n",
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0)).unwrap();
        let held = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_CLOEXEC)
            .open(&path)
            .unwrap();
        let old = temp.0.join("held.desktop");
        fs::rename(&path, &old).unwrap();
        fs::write(&path, b"replacement").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        set_owner_execute(&held, 0).unwrap();
        assert_eq!(fs::metadata(old).unwrap().mode() & 0o7777, 0o100);
        assert_eq!(fs::metadata(path).unwrap().mode() & 0o7777, 0o600);
    }

    #[test]
    fn recursive_file_links_return_an_error_instead_of_overflowing_host_stack() {
        let temp = Temp::new();
        let path = temp.0.join("cycle.desktop");
        fs::write(
            &path,
            format!("[Desktop Entry]\nType=Link\nURL={}\n", path_uri(&path)),
        )
        .unwrap();
        assert_eq!(
            open(&json!({"path":path}), &temp.context())
                .unwrap_err()
                .code,
            "bad_entry"
        );
        LINK_DEPTH.with(|depth| assert_eq!(depth.get(), 0));
    }
}

#[cfg(test)]
mod dry_tests {
    use super::*;
    #[test]
    #[ignore = "Run with private XDG directories and WIN11SHELL_DRY_RUN=1"]
    fn native_dry_validates_but_never_spawns_at_leaf_boundaries() {
        assert!(
            crate::backend::dry_run(),
            "This test must run in a separate dry process"
        );
        let root = std::env::temp_dir().join(format!(
            "win11-leaf-dry-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        let context = Context::new(root.clone(), root.clone(), "en".into(), false);
        let expected = json!({"ok":true,"dryRun":true});
        assert_eq!(
            launch_own("explorer", &json!({}), Lang::Nl).unwrap(),
            expected
        );
        assert_eq!(
            launch_installed("not-an-installed-dry-fixture.desktop", &[], None, Lang::Nl).unwrap(),
            expected
        );
        fs::write(root.join("note.txt"), "private fixture").unwrap();
        assert_eq!(
            open(&json!({"path":root.join("note.txt")}), &context).unwrap(),
            expected
        );
        assert_eq!(open_with(&json!({"path":root.join("note.txt"),"appId":"not-an-installed-dry-fixture.desktop"}),&context).unwrap(),expected);
        assert_eq!(open(&json!({"path":root}), &context).unwrap(), expected);
        assert_eq!(
            open(&json!({"path":root.join("missing")}), &context)
                .unwrap_err()
                .code,
            "not_found"
        );
        assert_eq!(
            open_uri("https://private.invalid/example", &context).unwrap(),
            expected
        );
        assert_eq!(
            open_uri("file:///definitely-not-a-real-dry-fixture", &context)
                .unwrap_err()
                .code,
            "not_found"
        );
        assert_eq!(
            open_uri("javascript:alert(1)", &context).unwrap_err().code,
            "bad_uri"
        );
        assert_eq!(open_uri("https://", &context).unwrap_err().code, "bad_uri");
        let launcher = root.join(format!(
            "untrusted-dry-fixture-{}.desktop",
            std::process::id()
        ));
        fs::write(
            &launcher,
            "[Desktop Entry]\nType=Application\nName=Private dry launcher\nExec=/bin/false\n",
        )
        .unwrap();
        assert_eq!(
            launch_desktop_file(&launcher, &[], &context)
                .unwrap_err()
                .code,
            "untrusted"
        );
        // If either final defense were missing, only this private fixture could be touched.
        let marker = root.join("must-not-be-created");
        let mut command = Command::new("/usr/bin/touch");
        command.arg(&marker);
        spawn(command).unwrap();
        assert!(!marker.exists());
        launch_gio(json!({"action":"unknown-dry-proof"}), None, Lang::Nl).unwrap();
    }
}
