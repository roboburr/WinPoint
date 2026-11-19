//! B0's mirror tests (notes/claude/settings-parity.md §7): both directions on a memory backend and a temporary
//! settings store, no loop, the start's adoption, and the accent through gnome_look's record.
use super::*;
use crate::settings::SettingsStore;
use std::cell::Cell;

fn memory() -> Live {
    Live::with_backend(Some(gio::memory_settings_backend_new()))
}

fn store() -> (SettingsStore, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("mirror-{}", crate::events::nonce()));
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
fn every_conversion_round_trips() {
    for (conv, winpoint, gnome) in [
        (Conv::Bool, json!(true), "true"),
        (Conv::Inverted, json!(true), "false"),
        (Conv::ClockFormat, json!(false), "'12h'"),
        (Conv::ClockFormat, json!(true), "'24h'"),
        (Conv::WeekStart, json!(-1), "'default'"),
        (Conv::WeekStart, json!(0), "'sunday'"),
        (Conv::WeekStart, json!(1), "'monday'"),
        (Conv::WeekStart, json!(6), "'saturday'"),
        (Conv::Percent, json!(125), "1.25"),
        (Conv::ReducedMotion, json!(true), "'reduce'"),
    ] {
        let v = to_gnome(conv, &winpoint).unwrap();
        assert_eq!(v.print(true).as_str(), gnome, "{conv:?} {winpoint}");
        assert_eq!(to_winpoint(conv, &v), Some(winpoint), "{conv:?}");
    }
    assert_eq!(to_gnome(Conv::WeekStart, &json!(7)), None);
    assert_eq!(to_gnome(Conv::Bool, &json!("yes")), None);
    assert_eq!(to_winpoint(Conv::ClockFormat, &"am".to_variant()), None);
}

/// Batch A's text scale pair (render.rs adds it at the merge).
static TEXT_SCALE: Pair = Pair {
    settings: &["accessibility.textScale"],
    schema: INTERFACE,
    key: "text-scaling-factor",
    conv: Conv::Percent,
};

#[test]
fn a_percent_is_an_integer_when_whole_and_never_written_back_rounded() {
    // GNOME's default 1.0 is settings.json's integer 100, so a start rewrites nothing (amendment 48)
    let (store, dir) = store();
    let defaults = store.snapshot().unwrap();
    let hundred = to_winpoint(Conv::Percent, &1.0f64.to_variant()).unwrap();
    assert!(hundred.is_i64(), "{hundred}");
    assert_eq!(hundred, defaults["accessibility"]["textScale"]);
    assert_eq!(
        to_winpoint(Conv::Percent, &1.25f64.to_variant()),
        Some(json!(125))
    );
    // more decimals stay, without the multiplication's binary noise
    assert_eq!(
        to_winpoint(Conv::Percent, &1.234f64.to_variant()),
        Some(json!(123.4))
    );
    assert_eq!(to_winpoint(Conv::Percent, &f64::NAN.to_variant()), None);
    assert_eq!(to_winpoint(Conv::Percent, &true.to_variant()), None);
    // the comparison is in WinPoint's domain: 123.4 agrees with 1.234, 125.0 with 125
    assert!(agrees(Conv::Percent, &json!(123.4), &1.234f64.to_variant()));
    assert!(agrees(Conv::Percent, &json!(125.0), &1.25f64.to_variant()));
    assert!(!agrees(Conv::Percent, &json!(123), &1.234f64.to_variant()));
    assert!(same(&json!(125), &json!(125.0)) && !same(&json!(125), &json!("125")));
    // on a memory backend: GNOME's 1.234 comes in as 123.4 and does not go back to GNOME
    let gnome = memory();
    if gnome.accepts(INTERFACE, "text-scaling-factor", &1.0f64.to_variant()) {
        let pairs = || std::iter::once(&TEXT_SCALE);
        let touched = changed(&["accessibility.textScale"]);
        assert_eq!(
            writes_of(pairs(), &defaults, &touched, &gnome).len(),
            0,
            "the default writes nothing"
        );
        assert!(gnome.write(INTERFACE, "text-scaling-factor", &1.234f64.to_variant()));
        let settings = patch(&store, json!({"accessibility": {"textScale": 123.4}}));
        assert_eq!(settings["accessibility"]["textScale"], json!(123.4));
        assert_eq!(
            writes_of(pairs(), &settings, &touched, &gnome).len(),
            0,
            "no write back"
        );
        assert!(agreement(&TEXT_SCALE, &settings, &gnome).is_some());
        let settings = patch(&store, json!({"accessibility": {"textScale": 125}}));
        let writes = writes_of(pairs(), &settings, &touched, &gnome);
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].1, 1.25f64.to_variant());
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn alt_tab_and_gnomes_app_switching_agree_while_settings_is_closed() {
    // batch D's review fix D-7: the pair keeps taskView.allDesktops and App Switching equal, inverted
    let (store, dir) = store();
    let gnome = memory();
    let switcher = "org.gnome.shell.app-switcher";
    if gnome.read(switcher, "current-workspace-only").is_none() {
        eprintln!("{switcher}: not installed here");
        return;
    }
    let settings = patch(&store, json!({"taskView": {"allDesktops": true}}));
    apply_with(&settings, &changed(&["taskView.allDesktops"]), &gnome);
    assert_eq!(
        gnome.read(switcher, "current-workspace-only"),
        Some(false.to_variant())
    );
    let settings = patch(&store, json!({"taskView": {"allDesktops": false}}));
    assert_eq!(
        apply_with(&settings, &changed(&["taskView"]), &gnome),
        1,
        "WinPoint's Taakweergave writes GNOME's key"
    );
    assert_eq!(
        gnome.read(switcher, "current-workspace-only"),
        Some(true.to_variant())
    );
    // GNOME Settings' App Switching (an outside write) comes back into settings.json
    assert!(gnome.write(switcher, "current-workspace-only", &false.to_variant()));
    assert_eq!(
        winpoint_patch(
            &settings,
            &gnome,
            Some((switcher, "current-workspace-only"))
        ),
        json!({"taskView": {"allDesktops": true}})
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn settings_json_to_gnome_and_back_without_a_loop() {
    let (store, dir) = store();
    let gnome = memory();
    // Runtime startup adopts GNOME's preference pairs first, including App Switching (H12).
    // Isolate the DND round trip from an unrelated initial desktop-preference difference.
    let initial = store.snapshot().unwrap();
    patch(&store, winpoint_patch(&initial, &gnome, None));
    // a WinPoint UI (the DND flyout) writes settings.json: the mirror writes GNOME's inverted key
    let settings = patch(&store, json!({"notifications": {"dnd": true}}));
    assert_eq!(
        apply_with(&settings, &changed(&["notifications.dnd"]), &gnome),
        1
    );
    assert_eq!(
        gnome.read("org.gnome.desktop.notifications", "show-banners"),
        Some(false.to_variant())
    );
    // the same change again writes nothing (equality guard), and GNOME's value adds nothing to settings.json
    assert_eq!(
        apply_with(&settings, &changed(&["notifications.dnd"]), &gnome),
        0
    );
    assert_eq!(winpoint_patch(&settings, &gnome, None), json!({}));
    // a Settings page writes GNOME's key (clock-format 12h): the watcher's patch brings settings.json along
    assert!(gnome.write(INTERFACE, "clock-format", &"12h".to_variant()));
    let p = winpoint_patch(&settings, &gnome, Some((INTERFACE, "clock-format")));
    assert_eq!(p, json!({"time": {"h24": false}}));
    let settings = patch(&store, p);
    assert_eq!(settings["time"]["h24"], false);
    // the settings change that follows writes GNOME nothing: no loop
    assert_eq!(apply_with(&settings, &changed(&["time.h24"]), &gnome), 0);
    assert_eq!(winpoint_patch(&settings, &gnome, None), json!({}));
    // one GNOME key, two settings: both follow GNOME, and the one that changed goes to GNOME
    assert!(gnome.write(INTERFACE, "clock-show-seconds", &true.to_variant()));
    let p = winpoint_patch(&settings, &gnome, None);
    assert_eq!(
        p,
        json!({"time": {"showSeconds": true}, "taskbar": {"showSeconds": true}})
    );
    patch(&store, p);
    let settings = patch(
        &store,
        json!({"taskbar": {"showSeconds": false}, "time": {"showSeconds": false}}),
    );
    assert_eq!(
        apply_with(&settings, &changed(&["taskbar.showSeconds"]), &gnome),
        1
    );
    assert_eq!(
        gnome.read(INTERFACE, "clock-show-seconds"),
        Some(false.to_variant())
    );
    // the first day: -1 is GNOME's 'default' and a valid settings value
    let settings = patch(&store, json!({"time": {"firstDayOfWeek": 0}}));
    assert_eq!(
        apply_with(&settings, &changed(&["time.firstDayOfWeek"]), &gnome),
        1
    );
    assert_eq!(
        gnome.read(CALENDAR, "week-start-day"),
        Some("sunday".to_variant())
    );
    assert!(gnome.write(CALENDAR, "week-start-day", &"default".to_variant()));
    let settings = patch(&store, winpoint_patch(&settings, &gnome, None));
    assert_eq!(settings["time"]["firstDayOfWeek"], -1);
    // a whole-object change touches every pair below it
    assert!(touched(&PAIRS[1], &changed(&["time"])).is_some());
    assert!(touched(&PAIRS[1], &changed(&["taskbar"])).is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_start_adopts_gnome_for_every_preference_pair() {
    // a change made in a GNOME or Ubuntu session wins at the next panel start (amendment 47)
    let (store, dir) = store();
    let gnome = memory();
    let settings = store.snapshot().unwrap();
    // GNOME's defaults are WinPoint's defaults, except Alt+Tab's: GNOME's App Switching shows every workspace,
    // WinPoint's taskView.allDesktops is Windows' "only the desktop I'm using"; GNOME wins (amendment 47, D-7)
    let all_workspaces = gnome
        .read("org.gnome.shell.app-switcher", "current-workspace-only")
        .and_then(|v| v.get::<bool>())
        .map(|only| !only);
    let expected = match all_workspaces {
        Some(all) if json!(all) != settings["taskView"]["allDesktops"] => {
            json!({"taskView": {"allDesktops": all}})
        }
        _ => json!({}),
    };
    let first = winpoint_patch(&settings, &gnome, None);
    assert_eq!(first, expected);
    let settings = patch(&store, first);
    assert_eq!(winpoint_patch(&settings, &gnome, None), json!({}));
    assert!(gnome.write(
        "org.gnome.desktop.privacy",
        "remember-recent-files",
        &false.to_variant()
    ));
    assert!(gnome.write(INTERFACE, "show-battery-percentage", &true.to_variant()));
    let adopt = winpoint_patch(&settings, &gnome, None);
    assert_eq!(
        adopt,
        json!({"privacy": {"recentFiles": false}, "taskbar": {"batteryPercent": true}})
    );
    let settings = patch(&store, adopt);
    assert_eq!(
        gnome_writes(&settings, &changed(&["privacy", "taskbar"]), &gnome).len(),
        0
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A GNOME side that counts the look writes (the accent goes through gnome_look::note, never a plain write).
struct Counting {
    inner: Live,
    looks: Cell<usize>,
}

impl Gnome for Counting {
    fn read(&self, schema: &str, key: &str) -> Option<glib::Variant> {
        self.inner.read(schema, key)
    }
    fn writable(&self, schema: &str, key: &str) -> bool {
        self.inner.writable(schema, key)
    }
    fn accepts(&self, schema: &str, key: &str, value: &glib::Variant) -> bool {
        self.inner.accepts(schema, key, value)
    }
    fn write(&self, schema: &str, key: &str, value: &glib::Variant) -> bool {
        assert_ne!(key, ACCENT_KEY, "the accent is a look key");
        self.inner.write(schema, key, value)
    }
    fn write_look(&self, schema: &str, key: &str, value: &glib::Variant) -> bool {
        self.looks.set(self.looks.get() + 1);
        self.inner.write(schema, key, value)
    }
}

#[test]
fn the_accent_goes_to_gnome_as_a_look_key_only_while_synced() {
    for (hex, name) in [
        ("#0067c0", "blue"),
        ("#0078d4", "blue"),
        ("#c42b1c", "red"),
        ("#107c10", "green"),
        ("#ca5010", "orange"),
        ("#e3008c", "pink"),
        ("#8764b8", "purple"),
        ("#038387", "teal"),
        ("#7a7574", "slate"),
        ("#ffb900", "yellow"),
    ] {
        assert_eq!(nearest_accent(hex), Some(name), "{hex}");
    }
    assert_eq!(nearest_accent("blue"), None);
    let gnome = Counting {
        inner: memory(),
        looks: Cell::new(0),
    };
    let synced = json!({"theme": {"syncSystem": true, "accent": "#c42b1c"}});
    assert_eq!(apply_with(&synced, &changed(&["theme.accent"]), &gnome), 1);
    assert_eq!(gnome.looks.get(), 1);
    assert_eq!(gnome.read(INTERFACE, ACCENT_KEY), Some("red".to_variant()));
    assert_eq!(
        apply_with(&synced, &changed(&["theme.accent"]), &gnome),
        0,
        "equal: no write"
    );
    let unsynced = json!({"theme": {"syncSystem": false, "accent": "#107c10"}});
    assert_eq!(
        apply_with(&unsynced, &changed(&["theme.accent"]), &gnome),
        0
    );
    assert_eq!(gnome.looks.get(), 1);
    // GNOME's accent never comes back into settings.json: it is a look pair, one way
    assert!(winpoint_patch(&synced, &gnome, None).get("theme").is_none());
}

#[test]
fn gnome_look_puts_the_accent_back_in_other_sessions() {
    // the record `win11-app --gnome-look` reads (gnome_look.rs), on a private backend and an in-memory record
    let backend = gio::memory_settings_backend_new();
    let schema = gio::SettingsSchemaSource::default()
        .unwrap()
        .lookup(INTERFACE, true)
        .unwrap();
    let s = gio::Settings::new_full(&schema, Some(&backend), None);
    s.set_string(ACCENT_KEY, "teal").unwrap();
    let mut record = json!({"version": 1, "keys": {}});
    assert!(crate::gnome_look::note_in(
        &mut record,
        &s,
        ACCENT_KEY,
        &"red".to_variant()
    ));
    s.set_string(ACCENT_KEY, "red").unwrap();
    assert!(crate::gnome_look::restore_keys(&record, |_| Some(
        s.clone()
    )));
    assert_eq!(s.string(ACCENT_KEY), "teal", "GNOME's own accent is back");
    // a value the user chose in GNOME after WinPoint wrote stays (the record already holds this write)
    crate::gnome_look::note_in(&mut record, &s, ACCENT_KEY, &"red".to_variant());
    s.set_string(ACCENT_KEY, "green").unwrap();
    assert!(crate::gnome_look::restore_keys(&record, |_| Some(
        s.clone()
    )));
    assert_eq!(s.string(ACCENT_KEY), "green");
}

#[test]
fn the_watcher_skips_the_mirrors_own_writes_once() {
    let v = "12h".to_variant();
    assert!(!is_echo("org.example.test", "k", &v));
    tag("org.example.test", "k", &v);
    assert!(is_echo("org.example.test", "k", &v));
    assert!(!is_echo("org.example.test", "k", &v), "consumed");
    tag("org.example.test", "k", &v);
    assert!(
        !is_echo("org.example.test", "k", &"24h".to_variant()),
        "another value is someone else's"
    );
    assert!(
        is_echo("org.example.test", "k", &v),
        "the 12h tag stays for its own change"
    );
}

#[test]
fn a_late_handler_consumes_the_older_tags_and_a_failed_write_leaves_none() {
    // review of B0, finding 5: DND on and off while the watcher is blocked (two tags), both handlers read the
    // latest value; a later external write of the first value must not be taken for the mirror's echo
    let (schema, key) = ("org.example.late", "show-banners");
    let (off, on) = (false.to_variant(), true.to_variant());
    tag(schema, key, &off);
    tag(schema, key, &on);
    assert!(is_echo(schema, key, &on), "handler 1 reads true");
    assert!(
        !is_echo(schema, key, &on),
        "handler 2 reads true: nothing left"
    );
    assert!(
        !is_echo(schema, key, &off),
        "Settings' own write of false, seconds later, is no echo"
    );
    // a write GNOME refused (a locked key, a wrong value) takes its tag back
    let gnome = Live::with_backend(Some(gio::null_settings_backend_new()));
    let banners = "org.gnome.desktop.notifications";
    let settings = json!({"notifications": {"dnd": true}});
    assert_eq!(
        apply_with(&settings, &changed(&["notifications.dnd"]), &gnome),
        0
    );
    assert!(!tagged(banners, "show-banners", &off, || false));
    assert!(
        !is_echo(banners, "show-banners", &off),
        "no tag of a write that did not happen"
    );
}

/// A GNOME without Ubuntu's brown accent (upstream gsettings-desktop-schemas), whose accent-color may be locked.
struct Upstream {
    inner: Counting,
    locked: Cell<bool>,
}

impl Gnome for Upstream {
    fn read(&self, schema: &str, key: &str) -> Option<glib::Variant> {
        self.inner.read(schema, key)
    }
    fn writable(&self, schema: &str, key: &str) -> bool {
        !(key == ACCENT_KEY && self.locked.get()) && self.inner.writable(schema, key)
    }
    fn accepts(&self, schema: &str, key: &str, value: &glib::Variant) -> bool {
        !(key == ACCENT_KEY && value.str() == Some("brown"))
            && self.inner.accepts(schema, key, value)
    }
    fn write(&self, schema: &str, key: &str, value: &glib::Variant) -> bool {
        self.inner.write(schema, key, value)
    }
    fn write_look(&self, schema: &str, key: &str, value: &glib::Variant) -> bool {
        assert!(
            self.accepts(schema, key, value) && self.writable(schema, key),
            "a look write GNOME cannot take is never tried (its record would be wrong)"
        );
        self.inner.write_look(schema, key, value)
    }
}

#[test]
fn the_accent_is_a_name_gnome_takes_and_never_goes_to_a_locked_key() {
    // review of B0, finding 3: WinPoint's palette brown on a GNOME without Ubuntu's brown
    assert_eq!(nearest_accent("#847545"), Some("brown"));
    let gnome = Upstream {
        inner: Counting {
            inner: memory(),
            looks: Cell::new(0),
        },
        locked: Cell::new(false),
    };
    let brown = json!({"theme": {"syncSystem": true, "accent": "#847545"}});
    let target = accent_write(&brown, &gnome).expect("the nearest name GNOME has");
    assert_eq!(
        target.str(),
        nearest_accent_in("#847545", |name| name != "brown")
    );
    assert_eq!(apply_with(&brown, &changed(&["theme.accent"]), &gnome), 1);
    assert_eq!(gnome.read(INTERFACE, ACCENT_KEY), Some(target));
    assert_eq!(
        apply_with(&brown, &changed(&["theme.accent"]), &gnome),
        0,
        "equal now"
    );
    // a locked accent-color gets no write and no record
    gnome.locked.set(true);
    let red = json!({"theme": {"syncSystem": true, "accent": "#c42b1c"}});
    assert_eq!(accent_write(&red, &gnome), None);
    assert_eq!(apply_with(&red, &changed(&["theme.accent"]), &gnome), 0);
    assert_eq!(gnome.inner.looks.get(), 1);
    // Live checks the type, the range and lockdown before gnome_look::note
    let live = memory();
    let nonsense = "nonsense-accent".to_variant();
    assert!(!live.accepts(INTERFACE, ACCENT_KEY, &nonsense));
    assert!(!live.write_look(INTERFACE, ACCENT_KEY, &nonsense));
    let record = std::env::var_os("XDG_CONFIG_HOME")
        .map(|d| std::path::PathBuf::from(d).join("win11-gnome/gnome-look.json"));
    if let Some(text) = record.and_then(|p| std::fs::read_to_string(p).ok()) {
        assert!(
            !text.contains("nonsense-accent"),
            "no record of a refused write"
        );
    }
    let null = Live::with_backend(Some(gio::null_settings_backend_new()));
    assert!(null.accepts(INTERFACE, ACCENT_KEY, &"red".to_variant()));
    assert!(
        !null.write_look(INTERFACE, ACCENT_KEY, &"red".to_variant()),
        "locked"
    );
}

/// The start with a record (review of B0, finding 4), on a memory backend and a temporary store.
fn start(store: &SettingsStore, gnome: &dyn Gnome, agreed: &mut Value) {
    let snapshot = || store.snapshot().ok();
    let apply = |p: Value| {
        patch(store, p);
    };
    let locked = |f: &mut dyn FnMut(&Value)| store.with_snapshot(|s| f(s)).is_ok();
    start_with(gnome, agreed, &snapshot, &apply, &locked);
}

#[test]
fn the_start_keeps_the_side_that_moved_since_both_last_agreed() {
    let (store, dir) = store();
    let gnome = memory();
    let mut agreed = json!({});
    // the first start: no record, GNOME wins (amendment 47), and both sides are recorded
    assert!(gnome.write(INTERFACE, "show-battery-percentage", &true.to_variant()));
    start(&store, &gnome, &mut agreed);
    assert_eq!(store.snapshot().unwrap()["taskbar"]["batteryPercent"], true);
    let id = format!("{INTERFACE} clock-show-seconds");
    assert_eq!(agreed["pairs"][&id]["gnome"], "false");
    assert_eq!(
        agreed["pairs"][&id]["winpoint"],
        json!({"time.showSeconds": false, "taskbar.showSeconds": false})
    );
    // the panel is down: the Settings app's own backend turns the clock seconds on in settings.json
    patch(
        &store,
        json!({"taskbar": {"showSeconds": true}, "time": {"showSeconds": true}}),
    );
    // and a GNOME session changes another pair
    assert!(gnome.write(
        "org.gnome.desktop.privacy",
        "remember-recent-files",
        &false.to_variant()
    ));
    start(&store, &gnome, &mut agreed);
    let settings = store.snapshot().unwrap();
    assert_eq!(
        settings["taskbar"]["showSeconds"], true,
        "settings.json's change stays"
    );
    assert_eq!(settings["time"]["showSeconds"], true);
    assert_eq!(
        gnome.read(INTERFACE, "clock-show-seconds"),
        Some(true.to_variant()),
        "and goes to GNOME"
    );
    assert_eq!(
        settings["privacy"]["recentFiles"], false,
        "GNOME's change comes in"
    );
    assert_eq!(agreed["pairs"][&id]["gnome"], "true");
    // both moved: GNOME wins, as at the first start
    patch(&store, json!({"time": {"firstDayOfWeek": 0}}));
    assert!(gnome.write(CALENDAR, "week-start-day", &"monday".to_variant()));
    start(&store, &gnome, &mut agreed);
    assert_eq!(store.snapshot().unwrap()["time"]["firstDayOfWeek"], 1);
    // nothing moved: nothing is written
    let before = store.snapshot().unwrap();
    let (adopt, pushes) = start_plan(&before, &gnome, &agreed);
    assert_eq!((adopt, pushes.len()), (json!({}), 0));
    // the record follows the running mirror too: a pair whose sides agree after a change
    let settings = patch(&store, json!({"notifications": {"dnd": true}}));
    apply_with(&settings, &changed(&["notifications.dnd"]), &gnome);
    assert!(remember(&mut agreed, all_pairs(), &settings, &gnome));
    assert_eq!(
        agreed["pairs"]["org.gnome.desktop.notifications show-banners"]["winpoint"],
        json!({"notifications.dnd": true})
    );
    // a pair whose sides differ (GNOME refused) keeps its old record
    let mut kept = agreed.clone();
    let differ = json!({"notifications": {"dnd": false}});
    assert!(!remember(&mut kept, all_pairs(), &differ, &gnome));
    let _ = std::fs::remove_dir_all(dir);
}
