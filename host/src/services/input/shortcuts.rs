//! `input.shortcuts*` (batch A, notes/claude/settings-parity.md §5.2, amendment 13): GNOME Settings' Keyboard
//! Shortcuts, for the page Sneltoetsen.
//!
//! * The catalogue is GNOME Settings' own: `/usr/share/gnome-control-center/keybindings/*.xml` (KeyListEntries:
//!   section name, schema, entries with an optional schema of their own and `hidden`), with [`BUILTIN`] (the files
//!   of gnome-control-center 50.3) when none is installed. Sections with the same name merge, as GNOME's do. The
//!   `hidden` entries (Super+1..9, the backward switchers) are not listed, as in GNOME Settings, but they count for
//!   conflicts and for Alles opnieuw instellen. WinPoint's own two (amendment 13: the extension's `task-view` and
//!   `task-manager`, which `Main.wm.addKeybinding` follows) are the section `winpoint`.
//! * Held accelerators (amendment 13): while the extension's `task-view` holds `<Alt>Tab`, it strips that from every
//!   `org.gnome.desktop.wm.keybindings` key and keeps the user's value in `task-view-restore`; `task-manager` does the
//!   same with Ctrl+Alt+Delete in media-keys `logout` (`logout-restore`). An item reports the accelerator held from
//!   it (`held: {accel, setting}`), and a write that would take one back is a conflict with WinPoint's key.
//! * `input.shortcuts.set {schema, key, accels, replace?}`: a single list as GNOME writes it (`[]` turns the shortcut
//!   off). An accelerator another shortcut has is a conflict: `{conflict: {schema, key, path, id, name, accel}}` and
//!   nothing written, unless `replace` (GNOME's Vervangen), which takes it from the other shortcut first.
//! * `reset` / `resetAll` are `g_settings_reset`, as GNOME's; Alles opnieuw instellen resets every GNOME key of the
//!   catalogue (WinPoint's two and the custom shortcuts are left, as GNOME leaves its custom ones).
//! * Custom shortcuts: media-keys `custom-keybindings` (paths) and the relocatable
//!   `org.gnome.settings-daemon.plugins.media-keys.custom-keybinding` at `.../custom-keybindings/custom<N>/` (name,
//!   command, binding), the first free `custom<N>` for a new one, its keys reset when it is removed (GNOME Settings'
//!   cc-keyboard-manager.c).
//!
//! The writes go through `gsettings::write_values_in` (the allow rows of `allow/input.rs`: the keybinding schemas
//! `Service("input.shortcuts.set")`, the custom child `Service("input.shortcuts.custom.update")`), so the B0 checks
//! (types, lockdown, the held accelerators, the forbidden schemas) apply. B0 has no service-side reset, so
//! [`reset_keys`] repeats `gsettings::reset`'s checks for a service caller (a note for I).
use crate::files::ApiError;
use crate::i18n::{Lang, NO_PARAMS};
use crate::services::gsettings::{self, Env, allow, owned};
use gio::prelude::*;
use serde_json::{Value, json};
use std::path::Path;

pub(crate) const WM: &str = "org.gnome.desktop.wm.keybindings";
pub(crate) const MUTTER: &str = "org.gnome.mutter.keybindings";
pub(crate) const WAYLAND: &str = "org.gnome.mutter.wayland.keybindings";
pub(crate) const SHELL: &str = "org.gnome.shell.keybindings";
pub(crate) const MEDIA: &str = "org.gnome.settings-daemon.plugins.media-keys";
pub(crate) const CUSTOM: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
pub(crate) const EXT: &str = owned::EXTENSION_SCHEMA;
pub(crate) const CUSTOM_DIR: &str =
    "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/";
const MAX_ACCELS: usize = 4;
const MAX_CUSTOM: usize = 256;

/// The section order of the page (GNOME's sections; WinPoint's first).
pub(crate) const SECTIONS: &[(&str, &str)] = &[
    ("winpoint", "WinPoint"),
    ("accessibility", "Accessibility"),
    ("launchers", "Launchers"),
    ("navigation", "Navigation"),
    ("screenshots", "Screenshots"),
    ("media", "Sound and Media"),
    ("system", "System"),
    ("typing", "Typing"),
    ("windows", "Windows"),
];

/// WinPoint's own shortcuts (extension/shortcutBridge.js): (key, GNOME-style name).
pub(crate) const WINPOINT: &[(&str, &str)] = &[
    ("task-view", "Open Task View"),
    ("task-manager", "Open Task Manager"),
];

/// gnome-control-center 50.3's keybinding files: (section, schema, key, description, hidden).
#[rustfmt::skip]
pub(crate) const BUILTIN: &[(&str, &str, &str, &str, bool)] = &[
    ("media", MEDIA, "volume-mute", "Volume mute/unmute", false),
    ("media", MEDIA, "volume-down", "Volume down", false),
    ("media", MEDIA, "volume-up", "Volume up", false),
    ("media", MEDIA, "mic-mute", "Microphone mute/unmute", false),
    ("media", MEDIA, "media", "Launch media player", false),
    ("media", MEDIA, "play", "Play (or play/pause)", false),
    ("media", MEDIA, "pause", "Pause playback", false),
    ("media", MEDIA, "stop", "Stop playback", false),
    ("media", MEDIA, "previous", "Previous track", false),
    ("media", MEDIA, "next", "Next track", false),
    ("media", MEDIA, "eject", "Eject", false),
    ("typing", WM, "switch-input-source", "Switch to next input source", false),
    ("typing", WM, "switch-input-source-backward", "Switch to previous input source", false),
    ("launchers", MEDIA, "help", "Launch help browser", false),
    ("launchers", MEDIA, "control-center", "Settings", false),
    ("launchers", MEDIA, "calculator", "Launch calculator", false),
    ("launchers", MEDIA, "email", "Launch email client", false),
    ("launchers", MEDIA, "terminal", "Launch terminal", false),
    ("launchers", MEDIA, "www", "Launch web browser", false),
    ("launchers", MEDIA, "home", "Home folder", false),
    ("launchers", MEDIA, "search", "Search", false),
    ("system", MEDIA, "logout", "Log out", false),
    ("system", MEDIA, "reboot", "Restart", false),
    ("system", MEDIA, "shutdown", "Power off", false),
    ("system", MEDIA, "screensaver", "Lock screen", false),
    ("accessibility", MEDIA, "magnifier", "Turn zoom on or off", false),
    ("accessibility", MEDIA, "magnifier-zoom-in", "Zoom in", false),
    ("accessibility", MEDIA, "magnifier-zoom-out", "Zoom out", false),
    ("accessibility", MEDIA, "screenreader", "Turn screen reader on or off", false),
    ("accessibility", MEDIA, "on-screen-keyboard", "Turn on-screen keyboard on or off", false),
    ("accessibility", MEDIA, "increase-text-size", "Increase text size", false),
    ("accessibility", MEDIA, "decrease-text-size", "Decrease text size", false),
    ("accessibility", MEDIA, "toggle-contrast", "High contrast on or off", false),
    ("launchers", SHELL, "switch-to-application-1", "Activate pinned app 1", true),
    ("launchers", SHELL, "switch-to-application-2", "Activate pinned app 2", true),
    ("launchers", SHELL, "switch-to-application-3", "Activate pinned app 3", true),
    ("launchers", SHELL, "switch-to-application-4", "Activate pinned app 4", true),
    ("launchers", SHELL, "switch-to-application-5", "Activate pinned app 5", true),
    ("launchers", SHELL, "switch-to-application-6", "Activate pinned app 6", true),
    ("launchers", SHELL, "switch-to-application-7", "Activate pinned app 7", true),
    ("launchers", SHELL, "switch-to-application-8", "Activate pinned app 8", true),
    ("launchers", SHELL, "switch-to-application-9", "Activate pinned app 9", true),
    ("launchers", SHELL, "open-new-window-application-1", "Open new window of pinned app 1", true),
    ("launchers", SHELL, "open-new-window-application-2", "Open new window of pinned app 2", true),
    ("launchers", SHELL, "open-new-window-application-3", "Open new window of pinned app 3", true),
    ("launchers", SHELL, "open-new-window-application-4", "Open new window of pinned app 4", true),
    ("launchers", SHELL, "open-new-window-application-5", "Open new window of pinned app 5", true),
    ("launchers", SHELL, "open-new-window-application-6", "Open new window of pinned app 6", true),
    ("launchers", SHELL, "open-new-window-application-7", "Open new window of pinned app 7", true),
    ("launchers", SHELL, "open-new-window-application-8", "Open new window of pinned app 8", true),
    ("launchers", SHELL, "open-new-window-application-9", "Open new window of pinned app 9", true),
    ("screenshots", SHELL, "show-screenshot-ui", "Take a screenshot interactively", false),
    ("screenshots", SHELL, "screenshot", "Take a screenshot", false),
    ("screenshots", SHELL, "screenshot-window", "Take a screenshot of a window", false),
    ("screenshots", SHELL, "show-screen-recording-ui", "Record a screencast interactively", false),
    ("system", SHELL, "toggle-message-tray", "Show the notification list", false),
    ("system", SHELL, "toggle-quick-settings", "Open the quick settings menu", false),
    ("system", SHELL, "focus-active-notification", "Focus the active notification", false),
    ("system", SHELL, "toggle-overview", "Show the overview", false),
    ("system", SHELL, "toggle-application-view", "Show all apps", false),
    ("navigation", WM, "move-to-workspace-1", "Move window to workspace 1", false),
    ("navigation", WM, "move-to-workspace-2", "Move window to workspace 2", false),
    ("navigation", WM, "move-to-workspace-3", "Move window to workspace 3", false),
    ("navigation", WM, "move-to-workspace-4", "Move window to workspace 4", false),
    ("navigation", WM, "move-to-workspace-last", "Move window to last workspace", false),
    ("navigation", WM, "move-to-workspace-left", "Move window one workspace to the left", false),
    ("navigation", WM, "move-to-workspace-right", "Move window one workspace to the right", false),
    ("navigation", WM, "move-to-workspace-up", "Move window one workspace up", true),
    ("navigation", WM, "move-to-workspace-down", "Move window one workspace down", true),
    ("navigation", WM, "move-to-monitor-left", "Move window one monitor to the left", false),
    ("navigation", WM, "move-to-monitor-right", "Move window one monitor to the right", false),
    ("navigation", WM, "move-to-monitor-up", "Move window one monitor up", false),
    ("navigation", WM, "move-to-monitor-down", "Move window one monitor down", false),
    ("navigation", WM, "switch-applications", "Switch applications", false),
    ("navigation", WM, "switch-applications-backward", "Switch to previous application", true),
    ("navigation", WM, "switch-windows", "Switch windows", false),
    ("navigation", WM, "switch-windows-backward", "Switch to previous window", true),
    ("navigation", WM, "switch-group", "Switch windows of an application", false),
    ("navigation", WM, "switch-group-backward", "Switch to previous window of an application", true),
    ("navigation", WM, "switch-panels", "Switch system controls", false),
    ("navigation", WM, "switch-panels-backward", "Switch to previous system control", true),
    ("navigation", WM, "cycle-windows", "Switch windows directly", false),
    ("navigation", WM, "cycle-windows-backward", "Switch directly to previous window", true),
    ("navigation", WM, "cycle-group", "Switch windows of an app directly", false),
    ("navigation", WM, "cycle-group-backward", "Switch directly to previous window of an app", true),
    ("navigation", WM, "cycle-panels", "Switch system controls directly", false),
    ("navigation", WM, "cycle-panels-backward", "Switch directly to previous system control", true),
    ("navigation", WM, "show-desktop", "Hide all normal windows", false),
    ("navigation", WM, "switch-to-workspace-1", "Switch to workspace 1", false),
    ("navigation", WM, "switch-to-workspace-2", "Switch to workspace 2", false),
    ("navigation", WM, "switch-to-workspace-3", "Switch to workspace 3", false),
    ("navigation", WM, "switch-to-workspace-4", "Switch to workspace 4", false),
    ("navigation", WM, "switch-to-workspace-last", "Switch to last workspace", false),
    ("navigation", WM, "switch-to-workspace-left", "Switch to workspace on the left", false),
    ("navigation", WM, "switch-to-workspace-right", "Switch to workspace on the right", false),
    ("navigation", WM, "switch-to-workspace-up", "Switch to workspace above", true),
    ("navigation", WM, "switch-to-workspace-down", "Switch to workspace below", true),
    ("system", WM, "panel-run-dialog", "Show the run command prompt", false),
    ("system", WAYLAND, "restore-shortcuts", "Restore the keyboard shortcuts", false),
    ("windows", WM, "activate-window-menu", "Activate the window menu", false),
    ("windows", WM, "toggle-fullscreen", "Toggle fullscreen mode", false),
    ("windows", WM, "toggle-maximized", "Toggle maximization state", false),
    ("windows", WM, "maximize", "Maximize window", false),
    ("windows", WM, "unmaximize", "Restore window", false),
    ("windows", WM, "close", "Close window", false),
    ("windows", WM, "minimize", "Hide window", false),
    ("windows", WM, "begin-move", "Move window", false),
    ("windows", WM, "begin-resize", "Resize window", false),
    ("windows", WM, "toggle-on-all-workspaces", "Toggle window on all workspaces or one", false),
    ("windows", WM, "raise-or-lower", "Raise window if covered, otherwise lower it", false),
    ("windows", WM, "raise", "Raise window above other windows", false),
    ("windows", WM, "lower", "Lower window below other windows", false),
    ("windows", WM, "maximize-vertically", "Maximize window vertically", false),
    ("windows", WM, "maximize-horizontally", "Maximize window horizontally", false),
    ("windows", MUTTER, "toggle-tiled-left", "View split on left", false),
    ("windows", MUTTER, "toggle-tiled-right", "View split on right", false),
];

/// One shortcut of the catalogue.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Entry {
    pub section: String,
    pub schema: String,
    pub key: String,
    pub name: String,
    pub hidden: bool,
}

/// The page's section id of a GNOME section name.
fn section_id(name: &str) -> String {
    SECTIONS
        .iter()
        .find(|(_, n)| *n == name)
        .map(|(id, _)| (*id).to_owned())
        .unwrap_or_else(|| {
            name.chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() {
                        c.to_ascii_lowercase()
                    } else {
                        '-'
                    }
                })
                .collect()
        })
}

fn schema_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 200
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

/// The entries of one KeyListEntries file (an entry's own `schema` before the file's).
pub(crate) fn parse_keylist(xml: &str) -> Vec<Entry> {
    let Some(doc) = super::layouts::parse_xml(xml) else {
        return Vec::new();
    };
    let list = doc.root_element();
    if !list.has_tag_name("KeyListEntries") {
        return Vec::new();
    }
    let section = section_id(list.attribute("name").unwrap_or(""));
    let schema = list.attribute("schema").unwrap_or("");
    list.children()
        .filter(|n| n.has_tag_name("KeyListEntry"))
        .filter_map(|e| {
            let key = e.attribute("name")?;
            let schema = e.attribute("schema").unwrap_or(schema);
            (schema_ok(schema) && gsettings_key_ok(key)).then(|| Entry {
                section: section.clone(),
                schema: schema.to_owned(),
                key: key.to_owned(),
                name: e.attribute("description").unwrap_or(key).to_owned(),
                hidden: e.attribute("hidden") == Some("true"),
            })
        })
        .collect()
}

fn gsettings_key_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// GNOME Settings' catalogue under `root` (its files, or [`BUILTIN`]), then WinPoint's two.
pub(crate) fn catalogue(root: &Path) -> Vec<Entry> {
    let mut files: Vec<_> =
        std::fs::read_dir(root.join("usr/share/gnome-control-center/keybindings"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "xml"))
            .collect();
    files.sort();
    let mut out: Vec<Entry> = Vec::new();
    for file in files {
        for entry in parse_keylist(&std::fs::read_to_string(file).unwrap_or_default()) {
            if !out
                .iter()
                .any(|o| o.schema == entry.schema && o.key == entry.key)
            {
                out.push(entry);
            }
        }
    }
    if out.is_empty() {
        out = BUILTIN
            .iter()
            .map(|(section, schema, key, name, hidden)| Entry {
                section: (*section).into(),
                schema: (*schema).into(),
                key: (*key).into(),
                name: (*name).into(),
                hidden: *hidden,
            })
            .collect();
    }
    out.extend(WINPOINT.iter().map(|(key, name)| Entry {
        section: "winpoint".into(),
        schema: EXT.into(),
        key: (*key).into(),
        name: (*name).into(),
        hidden: false,
    }));
    out
}

// --------------------------------------------------------------------------------------------- accelerators

/// An accelerator as GNOME compares them: its modifiers and its key, any spelling (`<Primary>` = `<Control>`,
/// `<Mod1>` = `<Alt>`, `<Mod4>` = `<Super>`, the key's case ignored).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Accel {
    pub mods: u8,
    pub key: String,
}

const SHIFT: u8 = 1;
const CONTROL: u8 = 2;
const ALT: u8 = 4;
const SUPER: u8 = 8;
const HYPER: u8 = 16;
const META: u8 = 32;

/// A GTK accelerator (`<Super><Shift>Page_Up`, `XF86AudioMute`); None for anything else (an unknown modifier, a key
/// name that is no keysym name, an empty key).
pub(crate) fn parse_accel(s: &str) -> Option<Accel> {
    if s.is_empty() || s.len() > 128 {
        return None;
    }
    let mut mods = 0u8;
    let mut rest = s;
    while let Some(after) = rest.strip_prefix('<') {
        let end = after.find('>')?;
        mods |= match after[..end].to_ascii_lowercase().as_str() {
            "shift" => SHIFT,
            "control" | "ctrl" | "primary" => CONTROL,
            "alt" | "mod1" => ALT,
            "super" | "mod4" => SUPER,
            "hyper" => HYPER,
            "meta" => META,
            _ => return None,
        };
        rest = &after[end + 1..];
    }
    let key_ok = !rest.is_empty()
        && rest.len() <= 64
        && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    let key = rest.to_ascii_lowercase();
    // the two spellings of the same keysym (xkb's Prior, GTK's Page_Up)
    let key = match key.as_str() {
        "prior" => "page_up".to_owned(),
        "next" => "page_down".to_owned(),
        "kp_prior" => "kp_page_up".to_owned(),
        "kp_next" => "kp_page_down".to_owned(),
        _ => key,
    };
    key_ok.then_some(Accel { mods, key })
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_owned))
        .collect()
}

fn has_accel(list: &[String], accel: &Accel) -> bool {
    list.iter().any(|a| parse_accel(a).as_ref() == Some(accel))
}

// ------------------------------------------------------------------------------------------------ the state

/// Who has an accelerator: a catalogued key or a custom shortcut.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Owner {
    pub schema: String,
    pub key: String,
    pub path: Option<String>,
    /// The custom shortcut's id (`custom3`), else None.
    pub id: Option<String>,
    pub name: String,
    pub accels: Vec<String>,
}

impl Owner {
    fn json(&self) -> Value {
        json!({"schema": self.schema, "key": self.key, "path": self.path, "id": self.id, "name": self.name})
    }
}

/// One custom shortcut.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Custom {
    pub id: String,
    pub path: String,
    pub name: String,
    pub command: String,
    pub binding: String,
}

/// `customN` of a custom shortcut's path (`.../custom-keybindings/custom3/`).
pub(crate) fn custom_id(path: &str) -> Option<&str> {
    let id = path.strip_prefix(CUSTOM_DIR)?.strip_suffix('/')?;
    let n = id.strip_prefix("custom")?;
    (!n.is_empty() && n.len() <= 4 && n.bytes().all(|b| b.is_ascii_digit())).then_some(id)
}

fn custom_path(id: &str) -> Option<String> {
    let path = format!("{CUSTOM_DIR}{id}/");
    custom_id(&path).is_some().then_some(path)
}

/// The first free `custom<N>` (GNOME's find_free_settings_path).
pub(crate) fn free_custom_id(paths: &[String]) -> Option<String> {
    (0..10_000)
        .map(|n| format!("custom{n}"))
        .find(|id| !paths.iter().any(|p| custom_id(p) == Some(id.as_str())))
}

/// The accelerator WinPoint holds from a key (amendment 13), from the extension's restore records: the user's
/// value (or the default) that the extension stripped. (key's current, its default, the extension's two values and
/// records.)
pub(crate) fn held_from(
    schema: &str,
    key: &str,
    default: &[String],
    task_view: &[String],
    task_view_restore: &Value,
    task_manager: &[String],
    logout_restore: &Value,
) -> Option<(String, &'static str)> {
    let first = |list: &[String], f: fn(&str) -> bool| list.iter().find(|a| f(a)).cloned();
    if schema == WM && task_view.iter().any(|a| owned::is_alt_tab(a)) {
        let record = &task_view_restore[key];
        if record.is_object() {
            let user = if record["user"].is_array() {
                strings(&record["user"])
            } else {
                default.to_vec()
            };
            return first(&user, owned::is_alt_tab).map(|a| (a, "task-view"));
        }
    }
    if schema == MEDIA
        && key == "logout"
        && task_manager.iter().any(|a| owned::is_ctrl_alt_delete(a))
        && logout_restore.is_object()
    {
        let user = if logout_restore["user"].is_array() {
            strings(&logout_restore["user"])
        } else {
            default.to_vec()
        };
        return first(&user, owned::is_ctrl_alt_delete).map(|a| (a, "task-manager"));
    }
    None
}

fn record(item: &Value) -> Value {
    item["value"]
        .as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .filter(Value::is_object)
        .unwrap_or(Value::Null)
}

/// Everything the page and the conflict check need: every catalogued key's item and the custom shortcuts.
pub(crate) struct State {
    pub entries: Vec<(Entry, Value)>,
    pub custom: Vec<Custom>,
    pub custom_paths: Vec<String>,
    /// The extension's restore records (`task-view-restore`, `logout-restore`; Null without them).
    pub task_view_restore: Value,
    pub logout_restore: Value,
}

pub(crate) fn state(env: &mut Env, root: &Path, lang: Lang) -> State {
    let mut entries = Vec::new();
    for entry in catalogue(root) {
        if let Ok(item) = gsettings::read_item_in(env, &entry.schema, None, &entry.key, lang)
            && item["available"] == true
        {
            entries.push((entry, item));
        }
    }
    let paths = gsettings::read_item_in(env, MEDIA, None, "custom-keybindings", lang)
        .map(|i| strings(&i["value"]))
        .unwrap_or_default();
    let mut custom = Vec::new();
    for path in &paths {
        let Some(id) = custom_id(path) else {
            continue;
        };
        let read = |env: &mut Env, key: &str| {
            gsettings::read_item_in(env, CUSTOM, Some(path), key, lang)
                .ok()
                .and_then(|i| i["value"].as_str().map(str::to_owned))
                .unwrap_or_default()
        };
        custom.push(Custom {
            id: id.to_owned(),
            path: path.clone(),
            name: read(env, "name"),
            command: read(env, "command"),
            binding: read(env, "binding"),
        });
    }
    let mut restore = |key: &str| {
        gsettings::read_item_in(env, EXT, None, key, lang)
            .map(|i| record(&i))
            .unwrap_or(Value::Null)
    };
    let (task_view_restore, logout_restore) =
        (restore("task-view-restore"), restore("logout-restore"));
    State {
        entries,
        custom,
        custom_paths: paths,
        task_view_restore,
        logout_restore,
    }
}

impl State {
    fn value_of(&self, schema: &str, key: &str) -> Option<&Value> {
        self.entries
            .iter()
            .find(|(e, _)| e.schema == schema && e.key == key)
            .map(|(_, i)| i)
    }

    /// Every owner of accelerators.
    fn owners(&self) -> Vec<Owner> {
        let mut out: Vec<Owner> = self
            .entries
            .iter()
            .map(|(e, item)| Owner {
                schema: e.schema.clone(),
                key: e.key.clone(),
                path: None,
                id: None,
                name: e.name.clone(),
                accels: strings(&item["value"]),
            })
            .collect();
        out.extend(self.custom.iter().map(|c| Owner {
            schema: CUSTOM.into(),
            key: "binding".into(),
            path: Some(c.path.clone()),
            id: Some(c.id.clone()),
            name: c.name.clone(),
            accels: if c.binding.is_empty() {
                Vec::new()
            } else {
                vec![c.binding.clone()]
            },
        }));
        out
    }

    /// The other owners of any of `accels` (not `me`), each with the accelerator it has.
    pub(crate) fn conflicts(&self, me: &Owner, accels: &[String]) -> Vec<(Owner, String)> {
        let mut out = Vec::new();
        for accel in accels.iter().filter_map(|a| parse_accel(a)) {
            for other in self.owners() {
                let same =
                    other.schema == me.schema && other.key == me.key && other.path == me.path;
                if !same && has_accel(&other.accels, &accel) {
                    let spelled = other
                        .accels
                        .iter()
                        .find(|a| parse_accel(a).as_ref() == Some(&accel))
                        .cloned()
                        .unwrap_or_default();
                    if !out.iter().any(|(o, _): &(Owner, String)| *o == other) {
                        out.push((other, spelled));
                    }
                }
            }
        }
        out
    }

    /// `input.shortcuts`' answer.
    pub(crate) fn json(&self) -> Value {
        let ext = |key: &str| {
            self.value_of(EXT, key)
                .map(|i| strings(&i["value"]))
                .unwrap_or_default()
        };
        let (tv, tm) = (ext("task-view"), ext("task-manager"));
        let (tv_restore, logout_restore) = (&self.task_view_restore, &self.logout_restore);
        let mut sections: Vec<Value> = Vec::new();
        for (id, title) in SECTIONS {
            let items: Vec<Value> = self
                .entries
                .iter()
                .filter(|(e, _)| e.section == *id && !e.hidden)
                .map(|(e, item)| {
                    let default = strings(&item["default"]);
                    let held = held_from(
                        &e.schema,
                        &e.key,
                        &default,
                        &tv,
                        tv_restore,
                        &tm,
                        logout_restore,
                    );
                    json!({
                        "schema": e.schema, "key": e.key, "path": Value::Null, "name": e.name,
                        "accels": item["value"], "default": item["default"], "isDefault": item["isDefault"],
                        "held": held.map(|(accel, setting)| json!({"accel": accel, "setting": setting})),
                        "editable": item["writable"] == true && item["type"] == "as" && item["mode"] == "service",
                    })
                })
                .collect();
            if !items.is_empty() {
                sections.push(json!({"id": id, "title": title, "items": items}));
            }
        }
        let custom: Vec<Value> = self
            .custom
            .iter()
            .map(|c| json!({"id": c.id, "path": c.path, "name": c.name, "command": c.command, "binding": c.binding}))
            .collect();
        json!({"sections": sections, "custom": custom})
    }
}

// ------------------------------------------------------------------------------------------------ the writes

fn bad(lang: Lang, key: &str) -> ApiError {
    ApiError::tr("bad_args", lang, key, NO_PARAMS)
}

/// The accelerators of a set: at most four valid ones, in order, each once.
pub(crate) fn parse_accels(v: &Value, lang: Lang) -> Result<Vec<String>, ApiError> {
    let list = v
        .as_array()
        .filter(|l| l.len() <= MAX_ACCELS)
        .ok_or_else(|| bad(lang, "input.badShortcut"))?;
    let mut out: Vec<String> = Vec::new();
    for a in list {
        let s = a.as_str().ok_or_else(|| bad(lang, "input.badShortcut"))?;
        let parsed = parse_accel(s).ok_or_else(|| bad(lang, "input.badShortcut"))?;
        if !out.iter().any(|o| parse_accel(o).as_ref() == Some(&parsed)) {
            out.push(s.to_owned());
        }
    }
    Ok(out)
}

/// A custom shortcut's binding: one accelerator, or "" (off).
fn parse_binding(v: &Value, lang: Lang) -> Result<String, ApiError> {
    match v.as_str() {
        Some("") => Ok(String::new()),
        Some(s) if parse_accel(s).is_some() => Ok(s.to_owned()),
        _ => Err(bad(lang, "input.badShortcut")),
    }
}

fn text_arg(v: &Value, max: usize, lang: Lang, key: &str) -> Result<String, ApiError> {
    let s = v.as_str().ok_or_else(|| bad(lang, key))?;
    let s = s.trim();
    if s.is_empty() || s.len() > max || s.chars().any(char::is_control) {
        return Err(bad(lang, key));
    }
    Ok(s.to_owned())
}

/// One step of a replacement. Validate every step before touching any existing binding.
struct WriteOp {
    method: &'static str,
    schema: String,
    path: Option<String>,
    values: Vec<(String, Value)>,
}

impl WriteOp {
    fn write(&self, env: &mut Env, lang: Lang, dry: bool) -> Result<Value, ApiError> {
        let values: Vec<_> = self
            .values
            .iter()
            .map(|(k, v)| (k.as_str(), v.clone()))
            .collect();
        gsettings::write_values_in(
            env,
            self.method,
            &self.schema,
            self.path.as_deref(),
            &values,
            lang,
            dry,
        )
    }
}

/// Take all requested accelerators from an owner in one write, keeping its unrelated bindings.
fn take_from(owner: &Owner, accels: &[String]) -> WriteOp {
    let targets: Vec<_> = accels.iter().filter_map(|a| parse_accel(a)).collect();
    let (method, key, value) = if owner.path.is_some() {
        ("input.shortcuts.custom.update", "binding", json!(""))
    } else {
        let left: Vec<_> = owner
            .accels
            .iter()
            .filter(|a| parse_accel(a).is_none_or(|a| !targets.contains(&a)))
            .cloned()
            .collect();
        ("input.shortcuts.set", owner.key.as_str(), json!(left))
    };
    WriteOp {
        method,
        schema: owner.schema.clone(),
        path: owner.path.clone(),
        values: vec![(key.into(), value)],
    }
}

/// Preflight the whole replacement, then apply it. Restore values and default status on a later failure.
/// GSettings offers no cross-schema transaction: a policy change during rollback can still prevent restoration;
/// report that explicitly instead of claiming the replacement was cleanly reverted.
fn apply_ops(env: &mut Env, ops: &[WriteOp], lang: Lang, dry: bool) -> Result<Value, ApiError> {
    let mut before = Vec::new();
    let mut preview = json!({});
    for op in ops {
        match op.write(env, lang, true) {
            Ok(v) => preview = v,
            Err(e) => {
                // The extension's held check uses current state. A validated earlier step releases that binding.
                let released = e.key.as_deref() == Some("gsettings.heldByWinPoint")
                    && e.data
                        .as_ref()
                        .and_then(|d| d["setting"].as_str())
                        .is_some_and(|key| {
                            ops[..before.len()].iter().any(|prior| {
                                prior.schema == EXT
                                    && prior.values.iter().any(|(k, v)| {
                                        k == key
                                            && v.as_array().is_some_and(|a| {
                                                a.iter().all(|v| {
                                                    v.as_str().is_some_and(|a| {
                                                        if key == "task-view" {
                                                            !owned::is_alt_tab(a)
                                                        } else {
                                                            !owned::is_ctrl_alt_delete(a)
                                                        }
                                                    })
                                                })
                                            })
                                    })
                            })
                        });
                if !released {
                    return Err(e);
                }
            }
        }
        let mut saved = Vec::new();
        for (key, _) in &op.values {
            saved.push(gsettings::read_item_in(
                env,
                &op.schema,
                op.path.as_deref(),
                key,
                lang,
            )?);
        }
        before.push(saved);
    }
    if dry {
        return Ok(preview);
    }
    let mut result = json!({});
    for (i, op) in ops.iter().enumerate() {
        match op.write(env, lang, false) {
            Ok(v) => result = v,
            Err(mut error) => {
                let mut rollback_failed = false;
                // Include the failing step: a backend may have accepted part of it.
                for j in (0..=i).rev() {
                    let prior = &ops[j];
                    for item in &before[j] {
                        let key = item["key"].as_str().unwrap_or("");
                        let restored = if item["isDefault"] == true {
                            reset_keys(
                                env,
                                prior.method,
                                &prior.schema,
                                prior.path.as_deref(),
                                &[key],
                                lang,
                                false,
                            )
                        } else {
                            gsettings::write_values_in(
                                env,
                                prior.method,
                                &prior.schema,
                                prior.path.as_deref(),
                                &[(key, item["value"].clone())],
                                lang,
                                false,
                            )
                        };
                        rollback_failed |= restored.is_err();
                    }
                }
                if rollback_failed {
                    let data = error.data.get_or_insert_with(|| json!({}));
                    data["rollbackFailed"] = json!(true);
                }
                return Err(error);
            }
        }
    }
    Ok(result)
}

fn conflict_json(found: &[(Owner, String)]) -> Value {
    let (owner, accel) = &found[0];
    let mut c = owner.json();
    c["accel"] = json!(accel);
    json!({ "conflict": c })
}

fn catalogued<'a>(state: &'a State, schema: &str, key: &str) -> Option<&'a (Entry, Value)> {
    state
        .entries
        .iter()
        .find(|(e, _)| e.schema == schema && e.key == key)
}

/// `input.shortcuts.set` on `env`.
pub(crate) fn set(
    env: &mut Env,
    root: &Path,
    args: &Value,
    lang: Lang,
    dry_run: bool,
) -> Result<Value, ApiError> {
    let (schema, key) = (
        args["schema"].as_str().unwrap_or(""),
        args["key"].as_str().unwrap_or(""),
    );
    if !args["path"].is_null() {
        return Err(bad(lang, "input.badShortcut"));
    }
    let accels = parse_accels(&args["accels"], lang)?;
    let st = state(env, root, lang);
    let Some((entry, item)) = catalogued(&st, schema, key) else {
        return Err(ApiError::tr(
            "not_found",
            lang,
            "input.noShortcut",
            NO_PARAMS,
        ));
    };
    let me = Owner {
        schema: entry.schema.clone(),
        key: entry.key.clone(),
        path: None,
        id: None,
        name: entry.name.clone(),
        accels: strings(&item["value"]),
    };
    let found = st.conflicts(&me, &accels);
    if !found.is_empty() {
        if args["replace"] != true {
            return Ok(conflict_json(&found));
        }
    }
    let mut ops: Vec<_> = found
        .iter()
        .map(|(owner, _)| take_from(owner, &accels))
        .collect();
    ops.push(WriteOp {
        method: "input.shortcuts.set",
        schema: schema.into(),
        path: None,
        values: vec![(key.into(), json!(accels))],
    });
    let mut out = apply_ops(env, &ops, lang, dry_run)?;
    out["replaced"] = json!(found.iter().map(|(o, _)| o.json()).collect::<Vec<_>>());
    Ok(out)
}

/// `gsettings::reset`'s checks for a service caller (B0's reset is the API's only): the key's allow row at the
/// schema's own or the given relocatable path in mode `Service(method)` or Write, not forbidden, installed and
/// writable; then `g_settings_reset` of each key.
pub(crate) fn reset_keys(
    env: &mut Env,
    method: &str,
    schema_id: &str,
    path: Option<&str>,
    keys: &[&str],
    lang: Lang,
    dry_run: bool,
) -> Result<Value, ApiError> {
    gsettings::reset_values_in(env, method, schema_id, path, keys, lang, dry_run)
}

/// `input.shortcuts.reset {schema, key}`.
pub(crate) fn reset(
    env: &mut Env,
    root: &Path,
    args: &Value,
    lang: Lang,
    dry_run: bool,
) -> Result<Value, ApiError> {
    let (schema, key) = (
        args["schema"].as_str().unwrap_or(""),
        args["key"].as_str().unwrap_or(""),
    );
    if !args["path"].is_null() {
        return Err(bad(lang, "input.badShortcut"));
    }
    if !catalogue(root)
        .iter()
        .any(|e| e.schema == schema && e.key == key)
    {
        return Err(ApiError::tr(
            "not_found",
            lang,
            "input.noShortcut",
            NO_PARAMS,
        ));
    }
    reset_keys(
        env,
        "input.shortcuts.set",
        schema,
        None,
        &[key],
        lang,
        dry_run,
    )
}

/// `input.shortcuts.resetAll {}`: every GNOME key of the catalogue that is installed and writable.
pub(crate) fn reset_all(
    env: &mut Env,
    root: &Path,
    lang: Lang,
    dry_run: bool,
) -> Result<Value, ApiError> {
    let mut count = 0;
    for entry in catalogue(root).iter().filter(|e| e.schema != EXT) {
        let Ok(item) = gsettings::read_item_in(env, &entry.schema, None, &entry.key, lang) else {
            continue;
        };
        if item["available"] != true || item["writable"] != true {
            continue;
        }
        match reset_keys(
            env,
            "input.shortcuts.set",
            &entry.schema,
            None,
            &[entry.key.as_str()],
            lang,
            dry_run,
        ) {
            Ok(_) => count += 1,
            Err(error) if error.key.as_deref() == Some("gsettings.heldByWinPoint") => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(json!({ "reset": count }))
}

fn custom_owner(path: &str, id: &str, name: &str) -> Owner {
    Owner {
        schema: CUSTOM.into(),
        key: "binding".into(),
        path: Some(path.into()),
        id: Some(id.into()),
        name: name.into(),
        accels: Vec::new(),
    }
}

/// `input.shortcuts.custom.add {name, command, binding, replace?}` → `{id, path}` or `{conflict}`.
pub(crate) fn custom_add(
    env: &mut Env,
    root: &Path,
    args: &Value,
    lang: Lang,
    dry_run: bool,
) -> Result<Value, ApiError> {
    let name = text_arg(&args["name"], 256, lang, "input.badName")?;
    let command = text_arg(&args["command"], 4096, lang, "input.badCommand")?;
    let binding = parse_binding(&args["binding"], lang)?;
    let st = state(env, root, lang);
    if st.custom_paths.len() >= MAX_CUSTOM {
        return Err(bad(lang, "input.badName"));
    }
    let id = free_custom_id(&st.custom_paths).ok_or_else(|| bad(lang, "input.badName"))?;
    let path = custom_path(&id).ok_or_else(|| bad(lang, "input.badName"))?;
    let me = custom_owner(&path, &id, &name);
    let found = st.conflicts(&me, std::slice::from_ref(&binding));
    if !found.is_empty() {
        if args["replace"] != true {
            return Ok(conflict_json(&found));
        }
    }
    let mut values = vec![
        ("name".into(), json!(name)),
        ("command".into(), json!(command)),
        ("binding".into(), json!(binding)),
    ];
    if env
        .schema(CUSTOM)
        .is_some_and(|s| s.has_key("enable-in-lockscreen"))
    {
        values.push(("enable-in-lockscreen".into(), json!(false)));
    }
    let mut ops: Vec<_> = found
        .iter()
        .map(|(owner, _)| take_from(owner, std::slice::from_ref(&binding)))
        .collect();
    ops.push(WriteOp {
        method: "input.shortcuts.custom.update",
        schema: CUSTOM.into(),
        path: Some(path.clone()),
        values,
    });
    let mut paths = st.custom_paths.clone();
    paths.push(path.clone());
    ops.push(WriteOp {
        method: "input.shortcuts.set",
        schema: MEDIA.into(),
        path: None,
        values: vec![("custom-keybindings".into(), json!(paths))],
    });
    apply_ops(env, &ops, lang, dry_run)?;
    Ok(json!({ "id": id, "path": path }))
}

fn listed<'a>(st: &'a State, args: &Value, lang: Lang) -> Result<&'a Custom, ApiError> {
    let id = args["id"].as_str().unwrap_or("");
    st.custom
        .iter()
        .find(|c| c.id == id)
        .ok_or_else(|| ApiError::tr("not_found", lang, "input.noShortcut", NO_PARAMS))
}

/// `input.shortcuts.custom.update {id, name?, command?, binding?, replace?}`.
pub(crate) fn custom_update(
    env: &mut Env,
    root: &Path,
    args: &Value,
    lang: Lang,
    dry_run: bool,
) -> Result<Value, ApiError> {
    let st = state(env, root, lang);
    let custom = listed(&st, args, lang)?.clone();
    let mut values: Vec<(&str, Value)> = Vec::new();
    let mut ops = Vec::new();
    if !args["name"].is_null() {
        values.push((
            "name",
            json!(text_arg(&args["name"], 256, lang, "input.badName")?),
        ));
    }
    if !args["command"].is_null() {
        values.push((
            "command",
            json!(text_arg(&args["command"], 4096, lang, "input.badCommand")?),
        ));
    }
    if !args["binding"].is_null() {
        let binding = parse_binding(&args["binding"], lang)?;
        let me = custom_owner(&custom.path, &custom.id, &custom.name);
        let found = st.conflicts(&me, std::slice::from_ref(&binding));
        if !found.is_empty() {
            if args["replace"] != true {
                return Ok(conflict_json(&found));
            }
        }
        ops.extend(
            found
                .iter()
                .map(|(owner, _)| take_from(owner, std::slice::from_ref(&binding))),
        );
        values.push(("binding", json!(binding)));
    }
    if values.is_empty() {
        return Err(bad(lang, "input.badName"));
    }
    ops.push(WriteOp {
        method: "input.shortcuts.custom.update",
        schema: CUSTOM.into(),
        path: Some(custom.path.clone()),
        values: values.into_iter().map(|(k, v)| (k.into(), v)).collect(),
    });
    apply_ops(env, &ops, lang, dry_run)
}

/// `input.shortcuts.custom.remove {id}`: out of the list, then its keys reset (GNOME Settings does both).
pub(crate) fn custom_remove(
    env: &mut Env,
    root: &Path,
    args: &Value,
    lang: Lang,
    dry_run: bool,
) -> Result<Value, ApiError> {
    let st = state(env, root, lang);
    let custom = listed(&st, args, lang)?.clone();
    let paths: Vec<String> = st
        .custom_paths
        .iter()
        .filter(|p| **p != custom.path)
        .cloned()
        .collect();
    let mut keys = vec!["name", "command", "binding"];
    if env
        .schema(CUSTOM)
        .is_some_and(|s| s.has_key("enable-in-lockscreen"))
    {
        keys.push("enable-in-lockscreen");
    }
    reset_keys(
        env,
        "input.shortcuts.custom.update",
        CUSTOM,
        Some(&custom.path),
        &keys,
        lang,
        true,
    )?;
    gsettings::write_values_in(
        env,
        "input.shortcuts.set",
        MEDIA,
        None,
        &[("custom-keybindings", json!(paths))],
        lang,
        dry_run,
    )?;
    reset_keys(
        env,
        "input.shortcuts.custom.update",
        CUSTOM,
        Some(&custom.path),
        &keys,
        lang,
        dry_run,
    )?;
    Ok(json!({ "removed": custom.id }))
}

#[cfg(test)]
mod replacement_tests {
    use super::*;
    use crate::services::gsettings::{Backend, Config};

    fn memory() -> Env {
        Env::new(
            &Config {
                backend: Backend::Memory,
                extension_dirs: Some(vec![]),
            },
            None,
        )
    }

    #[test]
    fn replacement_removes_every_requested_accelerator_from_one_owner() {
        let owner = Owner {
            schema: WM.into(),
            key: "close".into(),
            path: None,
            id: None,
            name: "Close".into(),
            accels: vec!["<Super>a".into(), "<Super>b".into(), "<Alt>F4".into()],
        };
        let op = take_from(&owner, &["<Super>A".into(), "<Mod4>b".into()]);
        assert_eq!(op.values, vec![("close".into(), json!(["<Alt>F4"]))]);
    }

    #[test]
    fn invalid_later_step_leaves_the_original_shortcut_untouched() {
        let mut env = memory();
        if !env.schema(WM).is_some_and(|s| s.has_key("close")) {
            return;
        }
        let before = gsettings::read_item_in(&mut env, WM, None, "close", Lang::En).unwrap();
        let ops = [
            WriteOp {
                method: "input.shortcuts.set",
                schema: WM.into(),
                path: None,
                values: vec![("close".into(), json!([]))],
            },
            WriteOp {
                method: "input.shortcuts.set",
                schema: WM.into(),
                path: None,
                values: vec![("close".into(), json!(42))],
            },
        ];
        assert!(apply_ops(&mut env, &ops, Lang::En, false).is_err());
        let after = gsettings::read_item_in(&mut env, WM, None, "close", Lang::En).unwrap();
        assert_eq!(after["value"], before["value"]);
        assert_eq!(after["isDefault"], before["isDefault"]);
    }

    #[test]
    fn reused_custom_path_never_inherits_lockscreen_permission() {
        let mut env = memory();
        if !env
            .schema(CUSTOM)
            .is_some_and(|s| s.has_key("enable-in-lockscreen"))
            || env.schema(MEDIA).is_none()
        {
            return;
        }
        let root = Path::new("/nonexistent-root");
        let args = json!({ "name": "First", "command": "true", "binding": "" });
        let first = custom_add(&mut env, root, &args, Lang::En, false).unwrap();
        let path = first["path"].as_str().unwrap();
        gsettings::write_values_in(
            &mut env,
            "input.shortcuts.custom.update",
            CUSTOM,
            Some(path),
            &[("enable-in-lockscreen", json!(true))],
            Lang::En,
            false,
        )
        .unwrap();
        custom_remove(
            &mut env,
            root,
            &json!({ "id": first["id"] }),
            Lang::En,
            false,
        )
        .unwrap();
        let removed = gsettings::read_item_in(
            &mut env,
            CUSTOM,
            Some(path),
            "enable-in-lockscreen",
            Lang::En,
        )
        .unwrap();
        assert_eq!(removed["value"], false);
        assert_eq!(removed["isDefault"], true);
        // Also defend against orphaned paths left by another application or an older WinPoint version.
        gsettings::write_values_in(
            &mut env,
            "input.shortcuts.custom.update",
            CUSTOM,
            Some(path),
            &[("enable-in-lockscreen", json!(true))],
            Lang::En,
            false,
        )
        .unwrap();
        let second = custom_add(
            &mut env,
            root,
            &json!({ "name": "Second", "command": "false", "binding": "" }),
            Lang::En,
            false,
        )
        .unwrap();
        assert_eq!(second["path"], first["path"]);
        let added = gsettings::read_item_in(
            &mut env,
            CUSTOM,
            Some(path),
            "enable-in-lockscreen",
            Lang::En,
        )
        .unwrap();
        assert_eq!(added["value"], false);
    }

    #[test]
    fn locked_custom_list_does_not_steal_the_lock_screen_shortcut() {
        let mut probe = memory();
        if probe.schema(MEDIA).is_none() || probe.schema(CUSTOM).is_none() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("shortcut-locks-{}", crate::events::nonce()));
        std::fs::create_dir_all(dir.join("defaults")).unwrap();
        std::fs::write(
            dir.join("defaults/locks"),
            "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings\n",
        )
        .unwrap();
        let mut env = Env::new(
            &Config {
                backend: Backend::Keyfile {
                    file: dir.join("keyfile"),
                    defaults: dir.join("defaults"),
                },
                extension_dirs: Some(vec![]),
            },
            None,
        );
        let original =
            gsettings::read_item_in(&mut env, MEDIA, None, "screensaver", Lang::En).unwrap();
        let binding = original["value"][0].as_str().unwrap();
        let error = custom_add(&mut env, Path::new("/nonexistent-root"),
            &json!({ "name": "Replacement", "command": "true", "binding": binding, "replace": true }),
            Lang::En, false).unwrap_err();
        assert_eq!(error.key.as_deref(), Some("gsettings.locked"));
        let after =
            gsettings::read_item_in(&mut env, MEDIA, None, "screensaver", Lang::En).unwrap();
        assert_eq!(after["value"], original["value"]);
        assert_eq!(after["isDefault"], original["isDefault"]);
        drop(env);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
