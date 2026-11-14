//! Apply the original wallpaper settings to GNOME's background; the native desktop
//! (win11-shell) draws the same settings itself. No GTK theme or other desktop preferences change.
use gio::prelude::*;
use serde_json::Value;
use std::path::{Path, PathBuf};

const SCHEMA: &str = "org.gnome.desktop.background";
const BUILTINS: &[&str] = &[
    "lotus-light",
    "lotus-dark",
    "flow-light",
    "flow-dark",
    "glow",
    "sunrise",
    "aurora",
    "dunes",
];
/// Built-in wallpapers that were renamed (old id, new id): settings saved before the rename
/// still name the old id, and GNOME's picture-uri is rewritten to the new file on the next sync.
const ALIASES: &[(&str, &str)] = &[("bloom-light", "lotus-light"), ("bloom-dark", "lotus-dark")];

/// The current id of a built-in wallpaper: an old (renamed) id becomes its new one.
pub(crate) fn builtin_id(id: &str) -> &str {
    ALIASES
        .iter()
        .find(|(old, _)| *old == id)
        .map_or(id, |(_, new)| new)
}

#[derive(Debug, PartialEq)]
struct Background {
    light: String,
    dark: String,
    fit: &'static str,
    color: String,
}

pub(crate) fn is_relevant(path: &str) -> bool {
    path == "wallpaper" || path.starts_with("wallpaper.") || matches!(path, "theme" | "theme.mode")
}

fn image(path: &str, mode: &str, follow: bool, ui: &Path) -> Result<String, String> {
    let file = if let Some(id) = path.strip_prefix("builtin:") {
        let id = builtin_id(id);
        if !BUILTINS.contains(&id) {
            return Err("Onbekende ingebouwde achtergrond.".into());
        }
        let id = if follow && (id.starts_with("lotus-") || id.starts_with("flow-")) {
            format!("{}-{mode}", id.split('-').next().unwrap())
        } else {
            id.to_owned()
        };
        ui.join("assets/wallpapers").join(format!("{id}.svg"))
    } else if path.starts_with("file:") {
        let url = url::Url::parse(path).map_err(|_| "Ongeldig lokaal afbeeldingsadres.")?;
        url.to_file_path()
            .map_err(|_| "De achtergrond moet een lokaal bestand zijn.")?
    } else if let Some(rest) = path.strip_prefix("~/") {
        glib::home_dir().join(rest)
    } else {
        PathBuf::from(path)
    };
    if !file.is_absolute() || !file.is_file() {
        return Err("De gekozen achtergrondafbeelding bestaat niet.".into());
    }
    Ok(gio::File::for_path(file).uri().to_string())
}

/// The CSS colour names (CSS Color 4), lower case.
const CSS_COLORS: &str = "aliceblue antiquewhite aqua aquamarine azure beige bisque black blanchedalmond blue \
    blueviolet brown burlywood cadetblue chartreuse chocolate coral cornflowerblue cornsilk crimson cyan darkblue \
    darkcyan darkgoldenrod darkgray darkgreen darkgrey darkkhaki darkmagenta darkolivegreen darkorange darkorchid \
    darkred darksalmon darkseagreen darkslateblue darkslategray darkslategrey darkturquoise darkviolet deeppink \
    deepskyblue dimgray dimgrey dodgerblue firebrick floralwhite forestgreen fuchsia gainsboro ghostwhite gold \
    goldenrod gray green greenyellow grey honeydew hotpink indianred indigo ivory khaki lavender lavenderblush \
    lawngreen lemonchiffon lightblue lightcoral lightcyan lightgoldenrodyellow lightgray lightgreen lightgrey \
    lightpink lightsalmon lightseagreen lightskyblue lightslategray lightslategrey lightsteelblue lightyellow lime \
    limegreen linen magenta maroon mediumaquamarine mediumblue mediumorchid mediumpurple mediumseagreen \
    mediumslateblue mediumspringgreen mediumturquoise mediumvioletred midnightblue mintcream mistyrose moccasin \
    navajowhite navy oldlace olive olivedrab orange orangered orchid palegoldenrod palegreen paleturquoise \
    palevioletred papayawhip peachpuff peru pink plum powderblue purple rebeccapurple red rosybrown royalblue \
    saddlebrown salmon sandybrown seagreen seashell sienna silver skyblue slateblue slategray slategrey snow \
    springgreen steelblue tan teal thistle tomato turquoise violet wheat white whitesmoke yellow yellowgreen";

/// A colour GDK 3's RGBA parser took (the check until GTK left the host, 14-11), so GNOME's primary-color gets what
/// it got before: `#` and 3, 6, 9 or 12 hex digits, `rgb(r,g,b)` or `rgba(r,g,b,a)` (numbers; the channels may be
/// percentages), or a colour name in any case and with spaces as Pango matched them ("dark slate gray"). An older
/// settings.json may hold a name; only the CSS names are known here, not the rest of X11's rgb.txt.
fn valid_color(spec: &str) -> bool {
    if let Some(hex) = spec.strip_prefix('#') {
        return matches!(hex.len(), 3 | 6 | 9 | 12) && hex.bytes().all(|b| b.is_ascii_hexdigit());
    }
    let (body, count) = match (spec.strip_prefix("rgba"), spec.strip_prefix("rgb")) {
        (Some(body), _) => (body, 4),
        (None, Some(body)) => (body, 3),
        (None, None) => {
            let name: String = spec.chars().filter(|c| *c != ' ').collect();
            let name = name.to_ascii_lowercase();
            return CSS_COLORS
                .split_ascii_whitespace()
                .any(|known| known == name);
        }
    };
    let Some(inner) = body
        .trim_start()
        .strip_prefix('(')
        .and_then(|rest| rest.trim_end().strip_suffix(')'))
    else {
        return false;
    };
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    parts.len() == count
        && parts.iter().enumerate().all(|(i, part)| {
            let number = match part.strip_suffix('%') {
                Some(number) if i < 3 => number.trim_end(),
                _ => part,
            };
            number.parse::<f64>().is_ok_and(f64::is_finite)
        })
}

fn target(settings: &Value, ui: &Path) -> Result<Background, String> {
    let wallpaper = &settings["wallpaper"];
    let color = wallpaper["color"].as_str().unwrap_or("#0b2a4a");
    if !valid_color(color) {
        return Err("Ongeldige achtergrondkleur.".into());
    }
    let color = color.to_owned();
    if wallpaper["type"] == "color" {
        return Ok(Background {
            light: String::new(),
            dark: String::new(),
            fit: "none",
            color,
        });
    }
    let path = wallpaper["path"]
        .as_str()
        .filter(|value| !value.is_empty())
        .unwrap_or("builtin:lotus-dark");
    let follow = wallpaper["followTheme"] != false;
    let mode = settings["theme"]["mode"].as_str().unwrap_or("dark");
    // Explicit Win11 choices work even with system-theme sync off. Automatic mode
    // uses both URI keys so GNOME follows its own light/dark mode immediately.
    let (light, dark) = match mode {
        "light" | "dark" => {
            let uri = image(path, mode, follow, ui)?;
            (uri.clone(), uri)
        }
        _ => (
            image(path, "light", follow, ui)?,
            image(path, "dark", follow, ui)?,
        ),
    };
    let fit = match wallpaper["fit"].as_str().unwrap_or("fill") {
        "fit" => "scaled",
        "stretch" => "stretched",
        "tile" => "wallpaper",
        "center" => "centered",
        "span" => "spanned",
        _ => "zoom",
    };
    Ok(Background {
        light,
        dark,
        fit,
        color,
    })
}

pub(crate) fn sync(settings: &Value, ui: &Path) -> Result<(), String> {
    let source =
        gio::SettingsSchemaSource::default().ok_or("GNOME-instellingenschema's ontbreken.")?;
    let schema = source
        .lookup(SCHEMA, true)
        .ok_or("Het GNOME-achtergrondschema ontbreekt.")?;
    let background = target(settings, ui)?;
    let settings = gio::Settings::new_full(&schema, gio::SettingsBackend::NONE, None);
    apply(&background, &settings)
}

fn apply(background: &Background, settings: &gio::Settings) -> Result<(), String> {
    // One grouped update prevents a flash of the old photo or a different fit.
    // Skip unchanged values: every independent app observes the shared store.
    settings.delay();
    for (key, value) in [
        ("picture-uri", background.light.as_str()),
        ("picture-uri-dark", background.dark.as_str()),
        ("picture-options", background.fit),
        ("primary-color", background.color.as_str()),
        ("secondary-color", background.color.as_str()),
        ("color-shading-type", "solid"),
    ] {
        // WinPoint's own session: GNOME's wallpaper is kept for the other sessions (gnome_look.rs)
        crate::gnome_look::note(settings, key, &value.to_variant());
        if settings.string(key).as_str() != value {
            settings
                .set_string(key, value)
                .map_err(|error| error.to_string())?;
        }
    }
    crate::gnome_look::note(settings, "picture-opacity", &100i32.to_variant());
    if settings.int("picture-opacity") != 100 {
        settings
            .set_int("picture-opacity", 100)
            .map_err(|error| error.to_string())?;
    }
    settings.apply();
    // Flush before releasing the store lock so an older asynchronous write cannot
    // arrive after another host has already applied the user's newer wallpaper.
    gio::Settings::sync();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    fn fixture() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "win11-wallpaper-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("assets/wallpapers")).unwrap();
        for id in BUILTINS {
            fs::write(
                root.join(format!("assets/wallpapers/{id}.svg")),
                b"<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'/>",
            )
            .unwrap();
        }
        root
    }
    #[test]
    fn original_builtin_follow_theme_modes_and_manual_path() {
        let root = fixture();
        for mode in ["light", "dark"] {
            for family in ["lotus", "flow"] {
                let result = target(&json!({"theme":{"mode":mode},"wallpaper":{"path":format!("builtin:{family}-dark")}}), &root).unwrap();
                assert!(result.light.ends_with(&format!("{family}-{mode}.svg")));
                assert_eq!(result.light, result.dark);
            }
        }
        let fixed = target(&json!({"theme":{"mode":"light"},"wallpaper":{"path":"builtin:lotus-dark","followTheme":false}}), &root).unwrap();
        assert!(fixed.light.ends_with("lotus-dark.svg"));
        let auto = target(
            &json!({"theme":{"mode":"system"},"wallpaper":{"path":"builtin:flow-dark"}}),
            &root,
        )
        .unwrap();
        assert!(auto.light.ends_with("flow-light.svg"));
        assert!(auto.dark.ends_with("flow-dark.svg"));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn renamed_bloom_ids_resolve_to_the_lotus_files() {
        let root = fixture();
        for mode in ["light", "dark"] {
            for old in ["bloom-light", "bloom-dark"] {
                let path = format!("builtin:{old}");
                let follow = target(
                    &json!({"theme":{"mode":mode},"wallpaper":{"path":path}}),
                    &root,
                )
                .unwrap();
                assert!(
                    follow.light.ends_with(&format!("/lotus-{mode}.svg")),
                    "{old} {mode}"
                );
                assert_eq!(follow.light, follow.dark);
                let fixed = target(
                    &json!({"theme":{"mode":mode},"wallpaper":{"path":path,"followTheme":false}}),
                    &root,
                )
                .unwrap();
                let new = old.replace("bloom", "lotus");
                assert!(
                    fixed.light.ends_with(&format!("/{new}.svg")),
                    "{old} {mode}"
                );
            }
        }
        assert_eq!(builtin_id("bloom-dark"), "lotus-dark");
        assert_eq!(builtin_id("flow-dark"), "flow-dark");
        assert!(target(&json!({"wallpaper":{"path":"builtin:bloom"}}), &root).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn all_six_fit_modes_and_solid_color_match_original_controls() {
        let root = fixture();
        for (fit, expected) in [
            ("fill", "zoom"),
            ("fit", "scaled"),
            ("stretch", "stretched"),
            ("tile", "wallpaper"),
            ("center", "centered"),
            ("span", "spanned"),
        ] {
            assert_eq!(
                target(&json!({"wallpaper":{"fit":fit}}), &root)
                    .unwrap()
                    .fit,
                expected
            );
        }
        let color = target(
            &json!({"wallpaper":{"type":"color","color":"#c04080"}}),
            &root,
        )
        .unwrap();
        assert_eq!(color.fit, "none");
        assert_eq!(color.color, "#c04080");
        assert!(color.light.is_empty() && color.dark.is_empty());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn local_photo_uri_escapes_special_characters_and_rejects_missing_or_remote_paths() {
        let root = fixture();
        let path = root.join("photo #1 %.png");
        fs::write(&path, b"image").unwrap();
        let result = target(&json!({"wallpaper":{"path":path}}), &root).unwrap();
        assert!(result.light.ends_with("photo%20%231%20%25.png"));
        assert_eq!(
            image(&result.light, "dark", false, &root).unwrap(),
            result.light
        );
        for path in [
            "https://example.com/photo.png",
            "relative.jpg",
            "builtin:../../secret",
            "/missing/photo.png",
            "file://remotehost/photo.png",
        ] {
            assert!(
                target(&json!({"wallpaper":{"path":path}}), &root).is_err(),
                "{path}"
            );
        }
        assert!(target(&json!({"wallpaper":{"color":"not a color"}}), &root).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn the_colours_gdk_3_took_and_the_css_names_are_valid() {
        for color in [
            "#0b2a4a",
            "#FFF",
            "#0b02a04a0",
            "#0b0b2a2a4a4a",
            "rgb(11,42,74)",
            "rgb ( 4%, 16 % ,29%) ",
            "rgba(11, 42, 74, 0.5)",
            "rgb(1.5,2,300)",
            "navy",
            "DarkSlateGray",
            "dark slate gray",
            "RebeccaPurple",
        ] {
            assert!(valid_color(color), "{color}");
        }
        for color in [
            "",
            "#",
            "#12",
            "#1234",
            "#12345g",
            "0b2a4a",
            "rgb(1,2)",
            "rgba(1,2,3)",
            "rgb(1,2,3,4)",
            "rgba(1,2,3,50%)",
            "rgb(1,2,x)",
            "rgb(inf,0,0)",
            "rgb(1,2,3",
            "RGB(1,2,3)",
            "hsl(0,0%,0%)",
            "transparent",
            "not a color",
        ] {
            assert!(!valid_color(color), "{color}");
        }
        assert_eq!(CSS_COLORS.split_ascii_whitespace().count(), 148);
    }
    #[test]
    fn only_wallpaper_and_mode_changes_request_a_desktop_update() {
        for path in [
            "wallpaper",
            "wallpaper.path",
            "wallpaper.color",
            "wallpaper.followTheme",
            "theme",
            "theme.mode",
        ] {
            assert!(is_relevant(path));
        }
        for path in [
            "theme.accent",
            "theme.gtkFamily",
            "language",
            "taskbar.alignment",
            "wallpaperOther",
        ] {
            assert!(!is_relevant(path));
        }
    }

    #[test]
    fn grouped_gnome_writes_are_visible_to_other_settings_clients() {
        let backend = gio::memory_settings_backend_new();
        let schema = gio::SettingsSchemaSource::default()
            .unwrap()
            .lookup(SCHEMA, true)
            .unwrap();
        let writer = gio::Settings::new_full(&schema, Some(&backend), None);
        let observer = gio::Settings::new_full(&schema, Some(&backend), None);
        let root = fixture();
        let photo = target(
            &json!({"theme":{"mode":"dark"},"wallpaper":{"path":"builtin:flow-light","fit":"fit"}}),
            &root,
        )
        .unwrap();
        writer.set_int("picture-opacity", 30).unwrap();
        apply(&photo, &writer).unwrap();
        assert!(observer.string("picture-uri").ends_with("flow-dark.svg"));
        assert_eq!(
            observer.string("picture-uri"),
            observer.string("picture-uri-dark")
        );
        assert_eq!(observer.string("picture-options"), "scaled");
        assert_eq!(observer.int("picture-opacity"), 100);
        let color = target(
            &json!({"wallpaper":{"type":"color","color":"#c04080"}}),
            &root,
        )
        .unwrap();
        apply(&color, &writer).unwrap();
        assert_eq!(observer.string("picture-options"), "none");
        assert_eq!(observer.string("primary-color"), "#c04080");
        assert_eq!(observer.string("secondary-color"), "#c04080");
        assert_eq!(observer.string("color-shading-type"), "solid");
        assert_eq!(observer.string("picture-uri"), "");
        apply(&photo, &writer).unwrap();
        assert_eq!(observer.string("picture-options"), "scaled");
        assert!(observer.string("picture-uri").ends_with("flow-dark.svg"));
        fs::remove_dir_all(root).unwrap();
    }
}
