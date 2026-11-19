//! Batch A's rendering pairs and effects (notes/claude/settings-parity.md §6.4, amendments 46, 48 and 49). Only
//! batch A edits this file.
//!
//! The pairs (allow rows `Mirrored` in services/gsettings/allow/input.rs; Settings' pages write the GNOME key, the
//! mirror keeps settings.json equal, and a settings.json change made by another WinPoint UI reaches GNOME):
//!
//! * `accessibility.highContrast` ↔ `org.gnome.desktop.a11y.interface high-contrast` (`Conv::Bool`);
//! * `accessibility.reduceMotion` ↔ `org.gnome.desktop.interface enable-animations` (`Conv::Inverted`) and ↔
//!   `org.gnome.desktop.a11y.interface reduced-motion` (`Conv::ReducedMotion`): GNOME Settings 50 writes
//!   reduced-motion, GNOME Shell and GTK still read enable-animations, so both keys follow the one setting (a change
//!   of either reaches settings.json, and the settings change writes the other key). enable-animations comes first:
//!   where both GNOME keys moved while the panel was down, the start's patch takes the later pair, so GNOME
//!   Settings' own key decides;
//! * `accessibility.textScale` ↔ `org.gnome.desktop.interface text-scaling-factor` (`Conv::Percent`, percent / 100,
//!   any value GNOME holds, 0.5-3.0 = 50-300 %; amendment 48: nothing is clamped on the way back). Only while
//!   `Conv::Percent` hands an integral percentage back as an integer ([`TEXT_SCALE_READY`]): B0's conversion makes
//!   GNOME's 1.0 the float 100.0, which never equals settings.json's integer default 100, so every start would
//!   rewrite textScale and B0's test "startup adoption aligns GNOME and WinPoint preferences" would fail. The fix is a patch for
//!   step I (notes/claude/settings-a.md, slice 3, note 13); the pair joins by itself once it is in.
//!
//! Nobody scales text twice: GTK 4.22 scales text sized in points by text-scaling-factor (through gtk-xft-dpi, from
//! the settings portal), not text sized in px, which is all of the kit's type ramp. So the kit, the shell and the
//! calendar scale their px ramp by GNOME's text-scaling-factor itself (read from GSettings, GTK's dpi next) instead
//! of multiplying settings.json's textScale on top of it (native/kit/src/theme.rs `System`).
//!
//! [`effects`]: the high-contrast rule of amendment 46. While `accessibility.highContrast` is true, WinPoint's Fluent
//! import block leaves ~/.config/gtk-4.0/gtk.css, so libadwaita and GTK apps show their own high-contrast style; when
//! it goes off the block comes back where WinPoint puts it (theme.syncSystem with a light or dark mode and a GTK
//! family other than "system", settings_effects.rs `sync_gtk4_block`).
use super::{Conv, Pair};
use crate::settings_runtime::effects::{gtk4_remove, sync_gtk4_block};
use glib::variant::ToVariant;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::{LazyLock, Mutex};

const INTERFACE: &str = "org.gnome.desktop.interface";
const A11Y: &str = "org.gnome.desktop.a11y.interface";

/// The text scale pair (module docs).
const TEXT_SCALE: Pair = Pair {
    settings: &["accessibility.textScale"],
    schema: INTERFACE,
    key: "text-scaling-factor",
    conv: Conv::Percent,
};

/// Whether `Conv::Percent` gives GNOME's 1.0 back as settings.json's integer 100 (module docs).
pub(crate) fn text_scale_ready() -> bool {
    super::to_winpoint(Conv::Percent, &1.0f64.to_variant()) == Some(json!(100))
}

/// [`text_scale_ready`] once (the pairs are fixed for the life of the process).
pub(crate) static TEXT_SCALE_READY: LazyLock<bool> = LazyLock::new(text_scale_ready);

/// Batch A's pairs (module docs), in this order.
pub(crate) static PAIRS: LazyLock<Vec<Pair>> = LazyLock::new(|| {
    let mut pairs = vec![
        Pair {
            settings: &["accessibility.highContrast"],
            schema: A11Y,
            key: "high-contrast",
            conv: Conv::Bool,
        },
        Pair {
            settings: &["accessibility.reduceMotion"],
            schema: INTERFACE,
            key: "enable-animations",
            conv: Conv::Inverted,
        },
        Pair {
            settings: &["accessibility.reduceMotion"],
            schema: A11Y,
            key: "reduced-motion",
            conv: Conv::ReducedMotion,
        },
    ];
    if *TEXT_SCALE_READY {
        pairs.push(TEXT_SCALE);
    }
    pairs
});

/// Whether a settings change touches `path` (equal to it, above it, below it, or everything).
fn touches(changed: &[String], path: &str) -> bool {
    changed.iter().any(|c| {
        c.is_empty()
            || c == path
            || path.starts_with(&format!("{c}."))
            || c.starts_with(&format!("{path}."))
    })
}

/// Amendment 46 in `config` (the user's config dir): the Fluent block out of gtk-4.0/gtk.css while high contrast
/// is on; back when it goes off (`was_on`: the value this host saw last) and WinPoint's look puts it there. The
/// action taken (settings_effects.rs' words: "removed", "deleted", "unchanged", "added", "no-fluent", ...), None
/// when nothing applies.
pub(crate) fn high_contrast_css(
    settings: &Value,
    was_on: Option<bool>,
    config: &Path,
) -> Option<String> {
    if settings["accessibility"]["highContrast"] == true {
        return Some(
            gtk4_remove(&config.join("gtk-4.0/gtk.css")).unwrap_or_else(|_| "error".to_owned()),
        );
    }
    let theme = &settings["theme"];
    let fluent = was_on == Some(true)
        && theme["syncSystem"] == true
        && matches!(theme["mode"].as_str(), Some("light" | "dark"))
        && theme["gtkFamily"] != "system";
    fluent.then(|| {
        sync_gtk4_block("fluent", config)["action"]
            .as_str()
            .unwrap_or("error")
            .to_owned()
    })
}

/// The high contrast this host saw last (None before the first change): the block comes back only when it goes
/// off, never on another settings change.
static LAST_HIGH_CONTRAST: Mutex<Option<bool>> = Mutex::new(None);

/// Batch A's rendering effects after a settings change (the panel host only, after the preference pairs).
pub(crate) fn effects(settings: &Value, changed: &[String]) {
    if !touches(changed, "accessibility.highContrast") {
        return;
    }
    let on = settings["accessibility"]["highContrast"] == true;
    let was_on = LAST_HIGH_CONTRAST
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .replace(on);
    if let Some(action) = high_contrast_css(settings, was_on, &glib::user_config_dir())
        && action == "error"
    {
        eprintln!("settings mirror: gtk-4.0/gtk.css did not follow high contrast");
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        Gnome, Live, all_pairs, apply_with, remember, start_plan, to_gnome, to_winpoint,
        winpoint_patch,
    };
    use super::*;
    use crate::settings::SettingsStore;

    fn memory() -> Live {
        Live::with_backend(Some(gio::memory_settings_backend_new()))
    }

    fn store() -> (SettingsStore, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("render-{}", crate::events::nonce()));
        std::fs::create_dir_all(&dir).unwrap();
        (SettingsStore::new(dir.clone()).unwrap(), dir)
    }

    fn patch(store: &SettingsStore, patch: Value) -> Value {
        store
            .call("settings.patch", &json!({ "patch": patch }))
            .unwrap()
            .unwrap()
            .0
    }

    fn changed(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|p| (*p).to_owned()).collect()
    }

    #[test]
    fn the_rendering_pairs_are_the_contracts() {
        let ids: Vec<(&str, &str, &str)> = PAIRS
            .iter()
            .map(|p| (p.settings[0], p.schema, p.key))
            .collect();
        assert_eq!(
            &ids[..3],
            &[
                ("accessibility.highContrast", A11Y, "high-contrast"),
                ("accessibility.reduceMotion", INTERFACE, "enable-animations"),
                ("accessibility.reduceMotion", A11Y, "reduced-motion"),
            ]
        );
        // the text scale joins exactly when Conv::Percent keeps integral percentages integers (module docs)
        assert_eq!(
            PAIRS.iter().any(|p| p.key == "text-scaling-factor"),
            text_scale_ready()
        );
        assert_eq!(all_pairs().filter(|p| p.schema == A11Y).count(), 2);
        // the text scale's conversion keeps every value of the schema's range, never a clamped one
        for (gnome, percent) in [(0.5, 50.0), (1.25, 125.0), (2.25, 225.0), (3.0, 300.0)] {
            let back = to_winpoint(Conv::Percent, &gnome.to_variant()).unwrap();
            assert_eq!(back.as_f64(), Some(percent));
            assert_eq!(to_gnome(Conv::Percent, &back), Some(gnome.to_variant()));
        }
    }

    #[test]
    fn reduced_motion_writes_both_gnome_keys_and_either_brings_the_setting() {
        let (store, dir) = store();
        let gnome = memory();
        // As at runtime startup, first align unrelated GNOME preferences (including App Switching).
        let initial = store.snapshot().unwrap();
        let settings = patch(&store, winpoint_patch(&initial, &gnome, None));
        assert_eq!(
            winpoint_patch(&settings, &gnome, None),
            json!({}),
            "startup adoption aligns GNOME and WinPoint preferences"
        );
        // another WinPoint UI turns motion down in settings.json: GNOME gets both keys
        let settings = patch(&store, json!({"accessibility": {"reduceMotion": true}}));
        assert_eq!(
            apply_with(&settings, &changed(&["accessibility.reduceMotion"]), &gnome),
            2
        );
        assert_eq!(
            gnome.read(A11Y, "reduced-motion"),
            Some("reduce".to_variant())
        );
        assert_eq!(
            gnome.read(INTERFACE, "enable-animations"),
            Some(false.to_variant())
        );
        // once more writes nothing (the equality guard)
        assert_eq!(
            apply_with(&settings, &changed(&["accessibility"]), &gnome),
            0
        );
        // GNOME Settings writes reduced-motion alone: the watcher's patch brings the setting, the settings change
        // that follows writes enable-animations to match, and nothing loops
        assert!(gnome.write(A11Y, "reduced-motion", &"no-preference".to_variant()));
        let p = winpoint_patch(&settings, &gnome, Some((A11Y, "reduced-motion")));
        assert_eq!(p, json!({"accessibility": {"reduceMotion": false}}));
        let settings = patch(&store, p);
        assert_eq!(
            apply_with(&settings, &changed(&["accessibility.reduceMotion"]), &gnome),
            1
        );
        assert_eq!(
            gnome.read(INTERFACE, "enable-animations"),
            Some(true.to_variant())
        );
        assert_eq!(
            winpoint_patch(&settings, &gnome, Some((INTERFACE, "enable-animations"))),
            json!({})
        );
        // a GNOME tool that turns animations off brings the setting too
        assert!(gnome.write(INTERFACE, "enable-animations", &false.to_variant()));
        assert_eq!(
            winpoint_patch(&settings, &gnome, Some((INTERFACE, "enable-animations"))),
            json!({"accessibility": {"reduceMotion": true}})
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_reduced_motion_set_in_a_gnome_session_comes_in_at_the_start() {
        let (store, dir) = store();
        let gnome = memory();
        // As at runtime startup, first align unrelated GNOME preferences (including App Switching).
        let initial = store.snapshot().unwrap();
        let settings = patch(&store, winpoint_patch(&initial, &gnome, None));
        let mut agreed = json!({});
        assert!(remember(&mut agreed, all_pairs(), &settings, &gnome));
        assert!(agreed["pairs"][format!("{A11Y} reduced-motion")].is_object());
        assert!(agreed["pairs"][format!("{INTERFACE} enable-animations")].is_object());
        // GNOME Settings, in a GNOME session, turns Reduced Motion on: only reduced-motion moves
        assert!(gnome.write(A11Y, "reduced-motion", &"reduce".to_variant()));
        let (adopt, pushes) = start_plan(&settings, &gnome, &agreed);
        assert_eq!(adopt, json!({"accessibility": {"reduceMotion": true}}));
        assert!(pushes.is_empty(), "enable-animations did not move");
        let settings = patch(&store, adopt);
        // the settings change after the adoption makes enable-animations agree
        assert_eq!(
            apply_with(&settings, &changed(&["accessibility.reduceMotion"]), &gnome),
            1
        );
        assert_eq!(
            gnome.read(INTERFACE, "enable-animations"),
            Some(false.to_variant())
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn high_contrast_follows_both_ways() {
        let (store, dir) = store();
        let gnome = memory();
        assert!(gnome.write(A11Y, "high-contrast", &true.to_variant()));
        let settings = store.snapshot().unwrap();
        let p = winpoint_patch(&settings, &gnome, Some((A11Y, "high-contrast")));
        assert_eq!(p, json!({"accessibility": {"highContrast": true}}));
        let settings = patch(&store, p);
        assert_eq!(
            apply_with(&settings, &changed(&["accessibility.highContrast"]), &gnome),
            0
        );
        // a WinPoint UI turns it off in settings.json: GNOME follows
        let settings = patch(&store, json!({"accessibility": {"highContrast": false}}));
        assert_eq!(
            apply_with(&settings, &changed(&["accessibility.highContrast"]), &gnome),
            1
        );
        assert_eq!(gnome.read(A11Y, "high-contrast"), Some(false.to_variant()));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn fluent_block(dir: &Path) -> Vec<u8> {
        let (dark, light) = (dir.join("dark.css"), dir.join("light.css"));
        std::fs::write(&dark, "/* dark */\n").unwrap();
        std::fs::write(&light, "/* light */\n").unwrap();
        crate::settings_runtime::effects::theme_block(Some(&dark), Some(&light))
    }

    #[test]
    fn high_contrast_takes_the_fluent_block_out_of_gtk_css() {
        let dir = std::env::temp_dir().join(format!("render-css-{}", crate::events::nonce()));
        let config = dir.join("config");
        std::fs::create_dir_all(config.join("gtk-4.0")).unwrap();
        let css = config.join("gtk-4.0/gtk.css");
        let mut text = fluent_block(&dir);
        text.extend_from_slice(b"\n/* the user's own */\nlabel { color: red; }\n");
        std::fs::write(&css, &text).unwrap();
        let theme = |family: &str, sync: bool| json!({"syncSystem": sync, "mode": "dark", "gtkFamily": family});
        let on = json!({"accessibility": {"highContrast": true}, "theme": theme("fluent", true)});
        assert_eq!(
            high_contrast_css(&on, None, &config).as_deref(),
            Some("removed")
        );
        let left = std::fs::read_to_string(&css).unwrap();
        assert!(!left.contains("win11shell theme"), "{left}");
        assert!(
            left.contains("label { color: red; }"),
            "the user's own CSS stays"
        );
        assert_eq!(
            high_contrast_css(&on, Some(true), &config).as_deref(),
            Some("unchanged")
        );
        // off with WinPoint's GTK family "system", or without syncSystem: nothing to put back
        for (family, sync) in [("system", true), ("fluent", false)] {
            let off =
                json!({"accessibility": {"highContrast": false}, "theme": theme(family, sync)});
            assert_eq!(
                high_contrast_css(&off, Some(true), &config),
                None,
                "{family} {sync}"
            );
        }
        // off while it was off (another change of the accessibility settings): the CSS is left alone
        let off = json!({"accessibility": {"highContrast": false}, "theme": theme("fluent", true)});
        assert_eq!(high_contrast_css(&off, Some(false), &config), None);
        assert_eq!(high_contrast_css(&off, None, &config), None);
        // going off with the Fluent family: sync_gtk4_block puts it back, or says no Fluent theme is installed
        let action = high_contrast_css(&off, Some(true), &config).unwrap();
        assert!(
            matches!(
                action.as_str(),
                "no-fluent" | "added" | "updated" | "created"
            ),
            "{action}"
        );
        // only a change that touches the setting runs the effect
        assert!(touches(
            &changed(&["accessibility"]),
            "accessibility.highContrast"
        ));
        assert!(touches(&changed(&[""]), "accessibility.highContrast"));
        assert!(!touches(
            &changed(&["accessibility.textScale"]),
            "accessibility.highContrast"
        ));
        let _ = std::fs::remove_dir_all(dir);
    }
}
