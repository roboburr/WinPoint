//! `update.options` and `update.setAutoOpen` (design §3.6 Geavanceerde opties, §4.12 D2).
//!
//! * `unattended`: `apt-config dump APT::Periodic::Unattended-Upgrade` (apt.systemd.daily's own key: "0" or unset
//!   = off, any other value, "always" too, = on); `null` when the program fails.
//! * `snaps`: `snap refresh --time --abs-time` (`timer:`/`schedule:`, `last:`, `hold:`, `next:` with RFC 3339 times
//!   or `n/a`) → `{last, next, held}`: `last`/`next` in ms or `null`; `held` while a `hold:` line holds refreshes
//!   (a time or `forever`), and then `next` is when they resume (`null` for forever), not snapd's "… (but held)"
//!   time. `null` when snap is missing or fails.
//! * Both programs come only from the tool table (`paths.rs`: the system path, or in test mode only the stub in
//!   `WIN11_UPDATE_TOOL_DIR`), with a cleared environment, no stdin, at most 5 s, both at once. They run only in a
//!   `settings` host that is not dry-run (design §4.11: no subprocess in dry-run; reader hosts are files only) and
//!   while Ubuntu's release upgrader does not run (it replaces them), else `null`.
//! * `autoOpen`: update-notifier's `com.ubuntu.update-notifier regular-auto-launch-interval` (days; it opens the
//!   Software Updater for regular updates after that many days, for security updates at once: the key's own
//!   description) through `gio::Settings` with a schema lookup: `{supported, on, days}`, `on` = below 36500.
//!   Production uses the default schema source and backend (dconf). Test mode uses only the fixture root's
//!   compiled schemas and a keyfile backend in it (`paths.rs` `gsettings_*`), and without a fixture root nothing:
//!   a test never reaches the real dconf. No schema (or no such int key): `supported:false`, the page hides the row.
//! * `update.setAutoOpen {on}` (never in dry-run: `update.rs` answers first): off writes 36500 and keeps the
//!   previous value in `settings.update.autoOpenPrevious` (the number the user had, or `"default"` when the key
//!   had no user value); on restores it (a number) or resets the key to its default (7), then clears the setting.
//!   A request that matches the current state changes nothing. Answers in the `update.options` shape with a fresh
//!   `autoOpen` and this host's last `unattended`/`snaps` rows (`null` before its first `update.options`): a write
//!   changes neither, and running the programs again would hold up a forwarded write's `update.reply` (review m6).
use super::{
    Shared,
    paths::{Setup, Tool, Tools},
};
use crate::{
    files::{ApiError, ApiResult},
    i18n::{Lang, NO_PARAMS},
};
use gio::prelude::*;
use serde_json::{Value, json};
use std::{
    io::Read,
    path::PathBuf,
    process::Stdio,
    sync::Mutex,
    time::{Duration, Instant},
};

pub(crate) const SCHEMA: &str = "com.ubuntu.update-notifier";
pub(crate) const KEY: &str = "regular-auto-launch-interval";
/// "Off": update-notifier waits 100 years (design §4.12).
pub(crate) const OFF_DAYS: i32 = 36500;
/// The setting that keeps the value from before "off".
const PREVIOUS: &str = "update.autoOpenPrevious";
const TIMEOUT: Duration = Duration::from_secs(5);
const MAX_OUTPUT: u64 = 64 << 10;

/// `{}` → `{unattended:bool|null, snaps:{last,next,held}|null, autoOpen:{supported, on, days}}`.
pub(super) fn options(shared: &Shared, _lang: Lang) -> ApiResult {
    Ok(view(shared))
}
fn view(shared: &Shared) -> Value {
    // both programs at once: the page waits for the slower one only (review m5)
    let (unattended, snaps) = match may_run(shared).then_some(&shared.setup.tools) {
        Some(tools) => std::thread::scope(|scope| {
            let apt = scope.spawn(|| unattended(tools));
            let snaps = snaps(tools);
            (apt.join().unwrap_or(None), snaps)
        }),
        None => (None, None),
    };
    let rows = (json!(unattended), json!(snaps));
    remember(shared, &rows);
    json!({"unattended":rows.0,"snaps":rows.1,"autoOpen":auto_open(&shared.setup)})
}
/// The last `unattended`/`snaps` rows per update service (its cache directory): `update.setAutoOpen`'s answer.
static ROWS: Mutex<Vec<(PathBuf, (Value, Value))>> = Mutex::new(Vec::new());
const ROWS_KEPT: usize = 8;
fn remember(shared: &Shared, rows: &(Value, Value)) {
    let mut kept = ROWS.lock().unwrap_or_else(|e| e.into_inner());
    kept.retain(|(dir, _)| *dir != shared.ctx.cache_dir);
    if kept.len() >= ROWS_KEPT {
        kept.remove(0);
    }
    kept.push((shared.ctx.cache_dir.clone(), rows.clone()));
}
/// `update.setAutoOpen`'s answer: a fresh `autoOpen`, the remembered rows, no program.
fn answer(shared: &Shared) -> Value {
    let kept = ROWS.lock().unwrap_or_else(|e| e.into_inner());
    let (unattended, snaps) =
        kept.iter().find(|(dir, _)| *dir == shared.ctx.cache_dir).map_or((Value::Null, Value::Null), |(_, rows)| rows.clone());
    drop(kept);
    json!({"unattended":unattended,"snaps":snaps,"autoOpen":auto_open(&shared.setup)})
}
/// Programs run only in a Settings host that is not dry-run, and not while Ubuntu's release upgrader replaces
/// them (snapd restarts during an upgrade; the rows then show "unknown" instead of waiting for the timeout).
fn may_run(shared: &Shared) -> bool {
    !(shared.ctx.dry_run || crate::backend::dry_run())
        && shared.ctx.kind == "settings"
        && !super::guard::upgrader_running(&shared.setup.paths)
}

/// `{on:bool}` (checked by `update.rs`) → the `update.options` shape (module doc). Never called in dry-run.
pub(super) fn set_auto_open(shared: &Shared, args: &Value, lang: Lang) -> ApiResult {
    // WU-FIX (GW4 review m8): update.tsv's own row (was sysmon.startup.saveFailed's text)
    let failed = |code: &str| ApiError::tr(code, lang, "update.options.err.autoOpen", NO_PARAMS);
    let Some(settings) = notifier(&shared.setup) else { return Err(failed("unsupported")) };
    let want = args["on"] == true;
    let days = settings.int(KEY);
    if want != (days < OFF_DAYS) {
        if want {
            let previous = shared.ctx.settings()["update"]["autoOpenPrevious"].clone();
            match previous.as_i64().filter(|n| (0..i64::from(OFF_DAYS)).contains(n)) {
                Some(n) => settings.set_int(KEY, n as i32).map_err(|_| failed("failed"))?,
                None => settings.reset(KEY),
            }
            // used up; a failure here only leaves a stale value that the next "off" overwrites
            let _ = shared.ctx.call("settings.set", &json!({"path":PREVIOUS,"value":null}));
        } else {
            let previous = if settings.user_value(KEY).is_some() { json!(days) } else { json!("default") };
            shared.ctx.call("settings.set", &json!({"path":PREVIOUS,"value":previous}))?;
            settings.set_int(KEY, OFF_DAYS).map_err(|_| failed("failed"))?;
        }
        // dconf writes asynchronously; the keyfile backend of test mode writes at once (and `sync` would create
        // the process's default backend)
        if !shared.setup.test {
            gio::Settings::sync();
        }
    }
    Ok(answer(shared))
}

/// update-notifier's settings object, or None (no schema, no int key, test mode without a fixture root).
fn notifier(setup: &Setup) -> Option<gio::Settings> {
    let (schema, backend) = if setup.test {
        let dir = setup.paths.gsettings_schemas.as_ref()?;
        let file = setup.paths.gsettings_keyfile.as_ref()?;
        if !dir.join("gschemas.compiled").is_file() {
            return None;
        }
        // not trusted: GLib checks the fixture's compiled file (review N4)
        let source = gio::SettingsSchemaSource::from_directory(dir, None, false).ok()?;
        (source.lookup(SCHEMA, false)?, Some(gio::keyfile_settings_backend_new(file.to_str()?, "/", None)))
    } else {
        (gio::SettingsSchemaSource::default()?.lookup(SCHEMA, true)?, None)
    };
    // a relocatable schema has no path, and g_settings_new_full aborts without one (review N4)
    if schema.path().is_none() || !schema.has_key(KEY) || schema.key(KEY).value_type().as_str() != "i" {
        return None;
    }
    Some(gio::Settings::new_full(&schema, backend.as_ref(), None))
}
fn auto_open(setup: &Setup) -> Value {
    match notifier(setup) {
        Some(settings) => {
            let days = settings.int(KEY).max(0);
            json!({"supported":true,"on":days < OFF_DAYS,"days":days})
        }
        None => json!({"supported":false,"on":false,"days":null}),
    }
}

/// apt.systemd.daily's switch for unattended-upgrades; None when apt-config fails.
fn unattended(tools: &Tools) -> Option<bool> {
    let (code, out) = run(tools, Tool::AptConfig, &["dump", "APT::Periodic::Unattended-Upgrade"])?;
    (code == 0).then(|| parse_unattended(&out))
}
/// `APT::Periodic::Unattended-Upgrade "1";` → on; "0", "00" or no such line → off (apt's default).
pub(crate) fn parse_unattended(out: &str) -> bool {
    out.lines()
        .filter_map(|line| line.trim().strip_prefix("APT::Periodic::Unattended-Upgrade "))
        .map(|value| value.trim().trim_end_matches(';').trim().trim_matches('"').trim().to_owned())
        .last()
        .is_some_and(|value| !value.is_empty() && !value.bytes().all(|c| c == b'0'))
}

/// snapd's refresh times; None when snap is missing or fails.
fn snaps(tools: &Tools) -> Option<Value> {
    let (code, out) = run(tools, Tool::Snap, &["refresh", "--time", "--abs-time"])?;
    if code != 0 {
        return None;
    }
    parse_snap_times(&out)
}
/// `snap refresh --time --abs-time` → `{last, next, held}` (module doc); None without a timer/schedule/last/next
/// line. snapd prints `hold:` only while refreshes are held (a time, or `forever`), and then `next: … (but held)`.
pub(crate) fn parse_snap_times(out: &str) -> Option<Value> {
    let (mut seen, mut last, mut next, mut hold, mut held) = (false, None, None, None, false);
    for line in out.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        let time = value.split_whitespace().next().and_then(iso_ms);
        match key.trim() {
            "timer" | "schedule" => seen = true,
            "last" => (seen, last) = (true, time),
            "next" => (seen, next) = (true, time),
            "hold" if !value.is_empty() && value != "n/a" => (held, hold) = (true, time),
            _ => {}
        }
    }
    if held {
        // the next refresh is when the hold ends (none for forever, review m9)
        next = hold.map(|h| next.map_or(h, |n: u64| n.max(h)));
    }
    seen.then(|| json!({"last":last,"next":next,"held":held}))
}
fn iso_ms(text: &str) -> Option<u64> {
    let dt = glib::DateTime::from_iso8601(text, None).ok()?;
    u64::try_from(dt.to_unix()).ok().map(|s| s * 1000)
}

/// Run a table program (no stdin, stdout only, at most [`TIMEOUT`]); `(exit code, stdout)` or None.
fn run(tools: &Tools, tool: Tool, args: &[&str]) -> Option<(i32, String)> {
    let mut command = tools.command(tool).ok()?;
    command.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = command.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = (&mut stdout).take(MAX_OUTPUT).read_to_end(&mut bytes);
        bytes
    });
    let started = Instant::now();
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) if started.elapsed() < TIMEOUT => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let bytes = reader.join().unwrap_or_default();
    Some((code?, String::from_utf8_lossy(&bytes).into_owned()))
}

// ------------------------------------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::{
        ServiceContext,
        update::{
            Mode, Shared, Stats, Update, paths, store,
            paths::{Env, VAR_FIXTURE_ROOT, VAR_TOOL_DIR},
            tests::{Fixture, script, write},
        },
    };
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::{Arc, atomic::AtomicU64},
    };

    fn fixtures() -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/update/fixtures/options");
        assert!(dir.join("schemas/gschemas.compiled").is_file(), "glib-compile-schemas tests/update/fixtures/options/schemas");
        dir
    }
    fn text(name: &str) -> String {
        fs::read_to_string(fixtures().join(name)).unwrap()
    }
    fn env_of(f: &Fixture, root: bool) -> Env {
        let (tools, fixture) = (f.tools.display().to_string(), f.root.display().to_string());
        let mut pairs = vec![(VAR_TOOL_DIR, tools.as_str())];
        if root {
            pairs.push((VAR_FIXTURE_ROOT, fixture.as_str()));
        }
        Env::fixture(&pairs, unsafe { libc::geteuid() })
    }
    /// A Shared for `kind`, with `settings.set` wired to a settings store on the context's own directory.
    fn shared(f: &Fixture, kind: &str, dry: bool, lang: &str, root: bool) -> Shared {
        let ctx = f.ctx(kind, dry, lang);
        wire_settings(&ctx);
        let store = store::Store::open(ctx.cache_dir.join("update")).unwrap();
        Shared { ctx, setup: paths::resolve(&env_of(f, root)).unwrap(), store, stats: Stats::default(), rev: AtomicU64::new(0) }
    }
    fn wire_settings(ctx: &Arc<ServiceContext>) {
        let settings = crate::settings::SettingsStore::new(ctx.config_dir.clone()).unwrap();
        ctx.set_caller(Arc::new(move |method, args| match settings.call(method, args) {
            Some(result) => result.map(|(value, _)| value),
            None => Err(ApiError::new("unknown_method", method)),
        }));
    }
    /// update-notifier's compiled schema inside the fixture root.
    fn install_schema(f: &Fixture) {
        let dir = f.root.join("usr/share/glib-2.0/schemas");
        fs::create_dir_all(&dir).unwrap();
        for name in ["gschemas.compiled", "com.ubuntu.update-notifier.gschema.xml"] {
            fs::copy(fixtures().join("schemas").join(name), dir.join(name)).unwrap();
        }
    }
    /// A stub that logs its whole argv and prints `out` (rc `rc`), with shell builtins only (its PATH is the stub
    /// directory alone).
    fn stub(f: &Fixture, name: &str, out: &str, rc: i32) {
        let data = f.base.join(format!("{name}.out"));
        fs::write(&data, out).unwrap();
        script(
            &f.tools.join(name),
            &format!("echo \"{name} $*\" >> '{}'\nwhile IFS= read -r line; do echo \"$line\"; done < '{}'\nexit {rc}\n", f.log.display(), data.display()),
        );
    }
    fn keyfile(f: &Fixture) -> String {
        fs::read_to_string(f.root.join("home/.config/glib-2.0/settings/keyfile")).unwrap_or_default()
    }
    fn iso(text: &str) -> u64 {
        iso_ms(text).unwrap()
    }

    #[test]
    fn apt_config_and_snap_outputs_are_parsed() {
        assert!(parse_unattended(&text("apt-config-on.txt")));
        assert!(!parse_unattended(&text("apt-config-off.txt")));
        assert!(!parse_unattended(&text("apt-config-unset.txt")), "unset is apt's default 0");
        assert!(parse_unattended("APT::Periodic::Unattended-Upgrade \"always\";\n"));
        assert!(!parse_unattended("APT::Periodic::Unattended-Upgrade \"00\";\n"));
        assert!(!parse_unattended("APT::Periodic::Unattended-Upgrade::Foo \"1\";\nAPT::Periodic::Update-Package-Lists \"1\";\n"), "another key");
        assert!(parse_unattended("APT::Periodic::Unattended-Upgrade \"0\";\nAPT::Periodic::Unattended-Upgrade \"1\";\n"), "the last one wins");
        let times = parse_snap_times(&text("snap-refresh-time.txt")).unwrap();
        assert_eq!(times, json!({"last":iso("2026-11-05T06:12:33Z"),"next":iso("2026-11-05T12:50:00Z"),"held":false}));
        assert_eq!(times["last"], 1_793_859_153_000u64, "07:12:33+01:00");
        let held = parse_snap_times(&text("snap-refresh-time-held.txt")).unwrap();
        assert_eq!(held, json!({"last":null,"next":iso("2026-11-30T23:00:00Z"),"held":true}));
        assert_eq!(parse_snap_times("schedule: 00:00-04:00\nlast: n/a\nnext: n/a\n"), Some(json!({"last":null,"next":null,"held":false})));
        // held: the next refresh is the end of the hold, not snapd's "(but held)" time; held forever: none
        let until = "timer: 00:00~24:00/4\nlast: 2026-11-05T07:12:33+01:00\nhold: 2026-12-01T00:00:00+01:00\nnext: 2026-11-05T13:50:00+01:00 (but held)\n";
        assert_eq!(parse_snap_times(until).unwrap()["next"], iso("2026-11-30T23:00:00Z"));
        let forever = "timer: 00:00~24:00/4\nlast: 2026-11-05T07:12:33+01:00\nhold: forever\nnext: 2026-11-05T13:50:00+01:00 (but held)\n";
        assert_eq!(parse_snap_times(forever), Some(json!({"last":iso("2026-11-05T06:12:33Z"),"next":null,"held":true})));
        assert_eq!(parse_snap_times("error: cannot communicate with server\n"), None);
        assert_eq!(parse_snap_times(""), None);
    }

    #[test]
    fn options_come_from_the_stubs_of_the_tool_table() {
        let f = Fixture::new("options");
        install_schema(&f);
        stub(&f, "apt-config", &text("apt-config-on.txt"), 0);
        stub(&f, "snap", &text("snap-refresh-time.txt"), 0);
        let shared = shared(&f, "settings", false, "nl", true);
        let result = options(&shared, Lang::Nl).unwrap();
        assert_eq!(result["unattended"], true);
        assert_eq!(result["snaps"], parse_snap_times(&text("snap-refresh-time.txt")).unwrap());
        assert_eq!(result["autoOpen"], json!({"supported":true,"on":true,"days":7}), "the schema default");
        let keys: Vec<&String> = result.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["unattended", "snaps", "autoOpen"]);
        // both at once: in either order
        let mut lines: Vec<String> = f.stub_log().lines().map(str::to_owned).collect();
        lines.sort();
        assert_eq!(lines, ["apt-config dump APT::Periodic::Unattended-Upgrade", "snap refresh --time --abs-time"]);
        assert_eq!(shared.setup.tools.runs(), 2);
        // setAutoOpen answers with these rows and starts no program (review m6)
        let set = set_auto_open(&shared, &json!({"on":false}), Lang::Nl).unwrap();
        assert_eq!((set["unattended"].clone(), set["snaps"].clone()), (result["unattended"].clone(), result["snaps"].clone()));
        assert_eq!(set["autoOpen"]["on"], false);
        assert_eq!(shared.setup.tools.runs(), 2);
        stub(&f, "apt-config", &text("apt-config-off.txt"), 0);
        assert_eq!(options(&shared, Lang::Nl).unwrap()["unattended"], false);
        // a program that fails (the fixture's default stubs exit 97), and a missing stub: unknown, no fallback
        stub(&f, "apt-config", "", 97);
        stub(&f, "snap", "error: cannot communicate with server: connection refused\n", 1);
        let failed = options(&shared, Lang::Nl).unwrap();
        assert_eq!((failed["unattended"].clone(), failed["snaps"].clone()), (Value::Null, Value::Null));
        fs::remove_file(f.tools.join("snap")).unwrap();
        let before = shared.setup.tools.runs();
        assert_eq!(options(&shared, Lang::Nl).unwrap()["snaps"], Value::Null);
        assert_eq!(shared.setup.tools.runs(), before + 1, "only apt-config started");
        assert_eq!(f.stub_log().lines().count(), 7, "{}", f.stub_log());
    }

    #[test]
    fn dry_run_and_reader_hosts_start_no_program() {
        let f = Fixture::new("options-dry");
        install_schema(&f);
        stub(&f, "apt-config", &text("apt-config-on.txt"), 0);
        stub(&f, "snap", &text("snap-refresh-time.txt"), 0);
        for (kind, dry) in [("settings", true), ("panel", false), ("start", false)] {
            let shared = shared(&f, kind, dry, "nl", true);
            let result = options(&shared, Lang::Nl).unwrap();
            assert_eq!(result, json!({"unattended":null,"snaps":null,"autoOpen":{"supported":true,"on":true,"days":7}}), "{kind}");
            assert_eq!(shared.setup.tools.runs(), 0, "{kind}");
        }
        // through the service: a dry Settings host and a panel answer the same, and setAutoOpen never writes
        let dry = Update::with_env(f.ctx("settings", true, "nl"), &env_of(&f, true));
        assert_eq!(dry.mode(), Mode::Dry);
        assert_eq!(dry.call("update.options", &json!({})).unwrap().unwrap()["unattended"], Value::Null);
        assert_eq!(dry.call("update.setAutoOpen", &json!({"on":false})).unwrap().unwrap(), json!({"dryRun":true}));
        dry.shutdown();
        let panel = Update::with_env(f.ctx("panel", false, "nl"), &env_of(&f, true));
        assert_eq!(panel.call("update.options", &json!({})).unwrap().unwrap()["autoOpen"]["supported"], true);
        assert_eq!(panel.call("update.setAutoOpen", &json!({"on":false})).unwrap().unwrap_err().code, "not_here");
        panel.shutdown();
        // while Ubuntu's release upgrader runs (a process of the fixture procfs), not even a Settings host starts them
        write(&f.root.join("proc/777/cmdline"), "python3\0do-release-upgrade\0");
        let upgrading = shared(&f, "settings", false, "nl", true);
        assert_eq!(options(&upgrading, Lang::Nl).unwrap()["snaps"], Value::Null);
        assert_eq!(upgrading.setup.tools.runs(), 0);
        assert_eq!(f.stub_log(), "");
        assert_eq!(keyfile(&f), "", "nothing was written");
    }

    #[test]
    fn auto_open_round_trips_in_a_keyfile_backend() {
        let f = Fixture::new("options-auto");
        install_schema(&f);
        let shared = shared(&f, "settings", false, "nl", true);
        let previous = |s: &Shared| s.ctx.settings()["update"]["autoOpenPrevious"].clone();
        // off: 36500 in the keyfile, the default remembered
        let off = set_auto_open(&shared, &json!({"on":false}), Lang::Nl).unwrap();
        assert_eq!(off["autoOpen"], json!({"supported":true,"on":false,"days":OFF_DAYS}));
        assert_eq!((off["unattended"].clone(), off["snaps"].clone()), (Value::Null, Value::Null), "no update.options yet");
        assert_eq!(shared.setup.tools.runs(), 0, "setAutoOpen starts no program");
        assert!(keyfile(&f).contains("[com/ubuntu/update-notifier]") && keyfile(&f).contains("regular-auto-launch-interval=36500"), "{}", keyfile(&f));
        assert_eq!(previous(&shared), "default");
        // a fresh settings object (a new backend on the same file) reads it back; off again changes nothing
        assert_eq!(options(&shared, Lang::Nl).unwrap()["autoOpen"]["on"], false);
        set_auto_open(&shared, &json!({"on":false}), Lang::Nl).unwrap();
        assert_eq!(previous(&shared), "default");
        // on: the key goes back to its default (7), the remembered value is used up
        let on = set_auto_open(&shared, &json!({"on":true}), Lang::Nl).unwrap();
        assert_eq!(on["autoOpen"], json!({"supported":true,"on":true,"days":7}));
        assert!(!keyfile(&f).contains("regular-auto-launch-interval="), "reset: {}", keyfile(&f));
        assert_eq!(previous(&shared), Value::Null);
        // the user's own value survives an off/on round trip
        fs::write(f.root.join("home/.config/glib-2.0/settings/keyfile"), "[com/ubuntu/update-notifier]\nregular-auto-launch-interval=3\n").unwrap();
        assert_eq!(options(&shared, Lang::Nl).unwrap()["autoOpen"], json!({"supported":true,"on":true,"days":3}));
        set_auto_open(&shared, &json!({"on":false}), Lang::Nl).unwrap();
        assert_eq!(previous(&shared), 3);
        assert_eq!(set_auto_open(&shared, &json!({"on":true}), Lang::Nl).unwrap()["autoOpen"]["days"], 3);
        assert!(keyfile(&f).contains("regular-auto-launch-interval=3"));
        // 0 = at once (on); a remembered value out of range falls back to the default
        fs::write(f.root.join("home/.config/glib-2.0/settings/keyfile"), "[com/ubuntu/update-notifier]\nregular-auto-launch-interval=0\n").unwrap();
        assert_eq!(options(&shared, Lang::Nl).unwrap()["autoOpen"], json!({"supported":true,"on":true,"days":0}));
        fs::write(f.root.join("home/.config/glib-2.0/settings/keyfile"), "[com/ubuntu/update-notifier]\nregular-auto-launch-interval=36500\n").unwrap();
        shared.ctx.call("settings.set", &json!({"path":PREVIOUS,"value":99999})).unwrap();
        assert_eq!(set_auto_open(&shared, &json!({"on":true}), Lang::Nl).unwrap()["autoOpen"]["days"], 7);
        // only the two read-only programs ran (the fixture's failing stubs: unknown rows), nothing else
        assert!(f.stub_log().lines().all(|l| l == "apt-config dump" || l == "snap refresh"), "{}", f.stub_log());
    }

    #[test]
    fn auto_open_is_unsupported_without_the_schema_and_never_uses_dconf_in_test_mode() {
        let f = Fixture::new("options-none");
        // WU-FIX (GW4 review m8): update.tsv's own row, not sysmon's
        for (lang, message) in [
            ("nl", "De instelling voor de Software-updater kon niet worden opgeslagen."),
            ("en", "The setting for Software Updater couldn’t be saved."),
        ] {
            let shared = shared(&f, "panel", false, lang, true);
            assert_eq!(options(&shared, Lang::Nl).unwrap()["autoOpen"], json!({"supported":false,"on":false,"days":null}));
            let error = set_auto_open(&shared, &json!({"on":false}), shared.ctx.lang()).unwrap_err();
            assert_eq!((error.code.as_str(), error.message.as_str()), ("unsupported", message));
            assert_eq!(error.key.as_deref(), Some("update.options.err.autoOpen"));
        }
        // test mode without a fixture root: no schema source, no backend at all (never the default, dconf)
        install_schema(&f);
        let bare = shared(&f, "panel", false, "nl", false);
        assert!(bare.setup.test && bare.setup.paths.gsettings_keyfile.is_none());
        assert_eq!(options(&bare, Lang::Nl).unwrap()["autoOpen"]["supported"], false);
        assert!(set_auto_open(&bare, &json!({"on":false}), Lang::Nl).is_err());
        // a schema directory without a compiled file
        fs::remove_file(f.root.join("usr/share/glib-2.0/schemas/gschemas.compiled")).unwrap();
        assert_eq!(options(&shared(&f, "panel", false, "nl", true), Lang::Nl).unwrap()["autoOpen"]["supported"], false);
        assert_eq!(keyfile(&f), "");
        assert_eq!(f.stub_log(), "");
    }

    #[test]
    fn the_two_page_toggles_persist_through_settings_set() {
        let f = Fixture::new("options-toggles");
        let ctx = f.ctx("settings", false, "nl");
        wire_settings(&ctx);
        assert_eq!(ctx.settings()["update"]["notifyRestart"], true, "default on");
        assert_eq!(ctx.settings()["update"]["metered"], false, "default off");
        ctx.call("settings.set", &json!({"path":"update.notifyRestart","value":false})).unwrap();
        ctx.call("settings.set", &json!({"path":"update.metered","value":true})).unwrap();
        // another host (a new store on the same file) sees both
        let again = crate::settings::SettingsStore::new(ctx.config_dir.clone()).unwrap().snapshot().unwrap();
        assert_eq!((again["update"]["notifyRestart"].clone(), again["update"]["metered"].clone()), (json!(false), json!(true)));
        let saved: Value = serde_json::from_slice(&fs::read(ctx.config_dir.join("settings.json")).unwrap()).unwrap();
        assert_eq!(saved["update"]["metered"], true);
        assert!(ctx.call("settings.set", &json!({"path":"update.metered","value":"yes"})).is_err(), "a toggle stays a bool");
    }
}
