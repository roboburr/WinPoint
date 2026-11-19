//! Batch A's host tests of slice 2 (notes/claude/settings-parity.md §7): the pages Typen, Taalopties, Geavanceerde
//! toetsenbordinstellingen and Sneltoetsen. Pure parsers on fixtures and on the installed files (skipped when
//! absent), the value mappings, the writes on a memory GSettings backend (dry runs write nothing), polkit and the
//! extension through in-process fakes on the sandbox's private session bus.
use super::capture;
use super::layouts::{self, parse_mo};
use super::shortcuts::{self, CUSTOM, EXT, MEDIA, WM, parse_accel};
use super::sources;
use super::*;
use crate::i18n::Lang;
use crate::services::gsettings::allow::{self, Mode};
use crate::services::gsettings::{Backend, Config, Env};
use gio::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

fn write(root: &Path, path: &str, data: &[u8]) {
    let p = root.join(path);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, data).unwrap();
}

fn temp(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("input-{tag}-{}", crate::events::nonce()));
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// A little-endian .mo file with `pairs` (and gettext's empty header entry).
fn mo_bytes(pairs: &[(&str, &str)]) -> Vec<u8> {
    let mut all: Vec<(&str, &str)> = vec![("", "Content-Type: text/plain; charset=UTF-8\n")];
    all.extend_from_slice(pairs);
    let n = all.len();
    let (orig, trans) = (28usize, 28 + 8 * n);
    let mut strings = Vec::new();
    let mut table_o = Vec::new();
    let mut table_t = Vec::new();
    let base = 28 + 16 * n;
    for (o, _) in &all {
        table_o.push((o.len(), base + strings.len()));
        strings.extend_from_slice(o.as_bytes());
        strings.push(0);
    }
    for (_, t) in &all {
        table_t.push((t.len(), base + strings.len()));
        strings.extend_from_slice(t.as_bytes());
        strings.push(0);
    }
    let mut out = Vec::new();
    for w in [
        0x9504_12de_u32,
        0,
        n as u32,
        orig as u32,
        trans as u32,
        0,
        0,
    ] {
        out.extend_from_slice(&w.to_le_bytes());
    }
    for (len, off) in table_o.into_iter().chain(table_t) {
        out.extend_from_slice(&(len as u32).to_le_bytes());
        out.extend_from_slice(&(off as u32).to_le_bytes());
    }
    out.extend_from_slice(&strings);
    out
}

#[test]
fn gettext_catalogues_translate_and_odd_files_are_refused() {
    let mo = parse_mo(&mo_bytes(&[
        ("Dutch", "Nederlands"),
        ("English (US)", "Engels (VS)"),
    ]))
    .unwrap();
    assert_eq!(mo.get("Dutch"), "Nederlands");
    assert_eq!(mo.get("German"), "German", "untranslated: the source text");
    assert!(parse_mo(b"not a catalogue at all, too short").is_none());
    let mut big = mo_bytes(&[("a", "b")]);
    big.swap(0, 3);
    big.swap(1, 2);
    assert!(
        parse_mo(&big).is_none(),
        "the magic of the other byte order with little-endian words"
    );
    let mut truncated = mo_bytes(&[("Dutch", "Nederlands")]);
    truncated.truncate(40);
    assert!(parse_mo(&truncated).is_some_and(|m| m.get("Dutch") == "Dutch"));
}

const RULES: &str = r#"<?xml version="1.0"?>
<xkbConfigRegistry version="1.1">
  <modelList/>
  <layoutList>
    <layout>
      <configItem>
        <name>us</name><shortDescription>en</shortDescription><description>English (US)</description>
        <countryList><iso3166Id>US</iso3166Id></countryList>
        <languageList><iso639Id>eng</iso639Id></languageList>
      </configItem>
      <variantList>
        <variant><configItem><name>intl</name><description>English (US, intl., with dead keys)</description></configItem></variant>
        <variant><configItem><name>cz_sk_de</name><description>Czech, Slovak and German (US)</description>
          <languageList><iso639Id>ces</iso639Id><iso639Id>slk</iso639Id><iso639Id>deu</iso639Id></languageList></configItem></variant>
        <variant><configItem><name>bad name!</name><description>skipped</description></configItem></variant>
      </variantList>
    </layout>
    <layout>
      <configItem>
        <name>nl</name><shortDescription>nl</shortDescription><description>Dutch</description>
        <countryList><iso3166Id>NL</iso3166Id></countryList>
        <languageList><iso639Id>nld</iso639Id></languageList>
      </configItem>
    </layout>
  </layoutList>
  <optionList/>
</xkbConfigRegistry>"#;

#[test]
fn xkb_rules_give_layouts_and_variants_with_inherited_languages() {
    let list = layouts::parse_rules(RULES);
    let ids: Vec<&str> = list.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(ids, ["us", "us+intl", "us+cz_sk_de", "nl"]);
    assert_eq!(
        list[1].languages,
        ["eng"],
        "a variant without languages has the layout's"
    );
    assert_eq!(list[1].countries, ["US"]);
    assert_eq!(list[1].short, "en");
    assert_eq!(list[2].languages, ["ces", "slk", "deu"]);
    assert!(layouts::parse_rules("<broken").is_empty());
}

#[test]
fn the_installed_xkb_rules_parse() {
    let Ok(xml) = std::fs::read_to_string("/usr/share/X11/xkb/rules/evdev.xml") else {
        eprintln!("skipped: no xkeyboard-config");
        return;
    };
    let list = layouts::parse_rules(&xml);
    assert!(list.len() > 300, "{}", list.len());
    for id in ["us", "us+intl", "nl", "de+nodeadkeys"] {
        assert!(list.iter().any(|l| l.id == id), "{id}");
    }
    assert!(
        list.iter().all(|l| sources::source_ok("xkb", &l.id)),
        "every id is one input.sources.set takes"
    );
}

fn iso_fixture(root: &Path) {
    write(
        root,
        "usr/share/iso-codes/json/iso_639-3.json",
        br#"{"639-3": [{"alpha_3": "eng", "alpha_2": "en", "name": "English"},
            {"alpha_3": "nld", "alpha_2": "nl", "bibliographic": "dut", "name": "Dutch"},
            {"alpha_3": "jpn", "alpha_2": "ja", "name": "Japanese"}]}"#,
    );
    write(
        root,
        "usr/share/iso-codes/json/iso_639-2.json",
        br#"{"639-2": [{"alpha_3": "nld", "name": "Dutch; Flemish"}, {"alpha_3": "ber", "name": "Berber languages"}]}"#,
    );
    write(
        root,
        "usr/share/iso-codes/json/iso_3166-1.json",
        br#"{"3166-1": [{"alpha_2": "NL", "name": "Netherlands"},
            {"alpha_2": "US", "name": "United States"}, {"alpha_2": "KR", "name": "Korea, Republic of", "common_name": "South Korea"}]}"#,
    );
    write(
        root,
        "usr/share/locale/nl/LC_MESSAGES/iso_639-3.mo",
        &mo_bytes(&[("Dutch", "Nederlands"), ("English", "Engels")]),
    );
    write(
        root,
        "usr/share/locale/nl/LC_MESSAGES/iso_3166-1.mo",
        &mo_bytes(&[("Netherlands", "Nederland")]),
    );
    // Ubuntu's langpack wins over the base catalogue
    write(
        root,
        "usr/share/locale-langpack/nl/LC_MESSAGES/iso_3166-1.mo",
        &mo_bytes(&[
            ("Netherlands", "Nederland"),
            ("United States", "Verenigde Staten"),
        ]),
    );
    write(
        root,
        "usr/share/locale/nl/LC_MESSAGES/xkeyboard-config.mo",
        &mo_bytes(&[("Dutch", "Nederlands")]),
    );
}

#[test]
fn layouts_carry_translated_languages_and_countries() {
    let root = temp("layouts");
    iso_fixture(&root);
    write(&root, "usr/share/X11/xkb/rules/evdev.xml", RULES.as_bytes());
    let nl = layouts::layouts(&root, Lang::Nl);
    let names: Vec<&str> = nl["layouts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["name"].as_str().unwrap())
        .collect();
    assert_eq!(names[0], "Czech, Slovak and German (US)", "sorted by name");
    assert!(
        names.contains(&"Nederlands"),
        "xkeyboard-config's catalogue: {names:?}"
    );
    assert!(
        names.contains(&"English (US, intl., with dead keys)"),
        "no translation: the English name"
    );
    let lang = |v: &Value, code: &str| {
        v["languages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["code"] == code)
            .map(|l| l["name"].clone())
    };
    assert_eq!(
        lang(&nl, "nld"),
        Some(json!("Nederlands")),
        "639-3 before 639-2's Dutch; Flemish"
    );
    assert_eq!(lang(&nl, "eng"), Some(json!("Engels")));
    assert_eq!(
        lang(&nl, "ces"),
        None,
        "not in the fixture's iso-codes: no name"
    );
    let country = |v: &Value, code: &str| {
        v["countries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["code"] == code)
            .map(|l| l["name"].clone())
    };
    assert_eq!(
        country(&nl, "US"),
        Some(json!("Verenigde Staten")),
        "the langpack"
    );
    let en = layouts::layouts(&root, Lang::En);
    assert_eq!(lang(&en, "nld"), Some(json!("Dutch")));
    assert_eq!(country(&en, "NL"), Some(json!("Netherlands")));
    let iso = layouts::IsoNames::load(&root, Lang::En);
    assert_eq!(iso.language("dut"), Some("Dutch"), "the bibliographic code");
    assert_eq!(
        iso.language("ja_JP"),
        Some("Japanese"),
        "IBus's language_REGION"
    );
    assert_eq!(
        iso.language("ber"),
        Some("Berber languages"),
        "639-2's collective code"
    );
    assert_eq!(iso.country("KR"), Some("South Korea"), "common_name first");
    let _ = std::fs::remove_dir_all(root);
}

const ANTHY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<component>
  <name>org.freedesktop.IBus.Anthy</name><textdomain>ibus-anthy</textdomain>
  <engines>
    <engine><name>anthy</name><language>ja</language><longname>Anthy</longname><layout>jp</layout></engine>
    <engine><name>xkb:jp::jpn</name><language>ja</language><longname>Japanese</longname><layout>jp</layout></engine>
    <engine><name>m17n:hi:itrans</name><language>hi</language><longname>itrans (m17n)</longname><layout>default</layout></engine>
    <engine><name>bad name</name><longname>Broken</longname></engine>
  </engines>
</component>"#;
const TABLE: &str = r#"<component><name>org.freedesktop.IBus.Table</name><engines exec='/usr/libexec/ibus-engine-table --xml'/></component>"#;

#[test]
fn ibus_components_give_their_inline_engines() {
    let list = layouts::parse_component(ANTHY);
    let ids: Vec<&str> = list.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(
        ids,
        ["anthy", "m17n:hi:itrans"],
        "no xkb: engines, no bad names"
    );
    assert_eq!(list[0].layout.as_deref(), Some("jp"));
    assert_eq!(list[1].layout, None, "layout default: the current one");
    assert!(
        layouts::parse_component(TABLE).is_empty(),
        "an engine list from a program is not run"
    );
    let root = temp("engines");
    iso_fixture(&root);
    write(
        &root,
        "usr/share/ibus/component/anthy.xml",
        ANTHY.as_bytes(),
    );
    write(
        &root,
        "usr/share/ibus/component/table.xml",
        TABLE.as_bytes(),
    );
    write(
        &root,
        "usr/share/ibus/component/notes.txt",
        b"not a component",
    );
    write(
        &root,
        "usr/share/applications/ibus-setup-anthy.desktop",
        b"[Desktop Entry]\n",
    );
    write(
        &root,
        "usr/share/locale/nl/LC_MESSAGES/ibus-anthy.mo",
        &mo_bytes(&[("Anthy", "Anthy (Japans)")]),
    );
    let v = layouts::engines(&root, Lang::Nl);
    let engines = v["engines"].as_array().unwrap();
    assert_eq!(engines.len(), 2);
    let anthy = engines.iter().find(|e| e["id"] == "anthy").unwrap();
    assert_eq!(
        anthy["name"], "Anthy (Japans)",
        "the component's textdomain"
    );
    assert_eq!(anthy["setup"], true);
    assert_eq!(anthy["languageName"], "Japanese");
    let itrans = engines
        .iter()
        .find(|e| e["id"] == "m17n:hi:itrans")
        .unwrap();
    assert_eq!(itrans["setup"], false, "no ibus-setup-m17n.desktop");
    for (id, ok) in [
        ("anthy", true),
        ("m17n:hi:itrans", true),
        ("a:b:c:d", false),
        ("", false),
        ("x y", false),
        ("mozc-jp", true),
    ] {
        assert_eq!(layouts::engine_id_ok(id), ok, "{id}");
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn input_sources_are_checked_and_synced_as_ubuntus_helper_takes_them() {
    let lang = Lang::En;
    let ok = sources::parse_sources(&json!({"sources": [["xkb", "us+intl"], ["ibus", "anthy"], ["xkb", "us+intl"], ["xkb", "nl"]]}), lang).unwrap();
    assert_eq!(
        ok,
        [
            ("xkb".into(), "us+intl".into()),
            ("ibus".into(), "anthy".into()),
            ("xkb".into(), "nl".into())
        ],
        "a duplicate dropped"
    );
    for bad in [
        json!({}),
        json!({"sources": [["xkb"]]}),
        json!({"sources": [["xkb", "us", "x"]]}),
        json!({"sources": [["evdev", "us"]]}),
        json!({"sources": [["xkb", "us+intl+x"]]}),
        json!({"sources": [["xkb", "../us"]]}),
        json!({"sources": [["ibus", "a b"]]}),
    ] {
        let err = sources::parse_sources(&bad, lang).unwrap_err();
        assert_eq!(
            (err.code.as_str(), err.key.as_deref()),
            ("bad_args", Some("input.badSources")),
            "{bad}"
        );
    }
    assert_eq!(
        sources::debian_args(&ok, &["compose:ralt".into(), "lv3:ralt_switch".into()]),
        Some(vec![
            "us,nl".into(),
            "intl,".into(),
            "compose:ralt,lv3:ralt_switch".into()
        ]),
        "<layouts> <variants> [options], the IBus engine left out"
    );
    assert_eq!(
        sources::debian_args(&ok, &[]).unwrap().len(),
        2,
        "no options: no third argument"
    );
    assert_eq!(
        sources::debian_args(&[("ibus".into(), "anthy".into())], &[]),
        None,
        "no xkb source: nothing to sync"
    );
    assert_eq!(
        sources::debian_args(&[("xkb".into(), "us".into())], &["grp:alt shift".into()]),
        None,
        "a character the helper refuses: no sync"
    );
    // the special character keys keep every other option
    let current: Vec<String> = ["caps:ctrl_modifier", "lv3:lalt_switch", "compose:caps"]
        .map(String::from)
        .to_vec();
    assert_eq!(
        sources::merge_options(&current, Some(Some("lv3:ralt_alt")), None),
        ["caps:ctrl_modifier", "compose:caps", "lv3:ralt_alt"]
    );
    assert_eq!(
        sources::merge_options(&current, Some(None), Some(None)),
        ["caps:ctrl_modifier"]
    );
    assert_eq!(sources::merge_options(&current, None, None), current);
    assert!(
        sources::preview_ok("us+intl")
            && !sources::preview_ok("us intl")
            && !sources::preview_ok("")
    );
}

fn memory() -> Env {
    Env::new(
        &Config {
            backend: Backend::Memory,
            extension_dirs: Some(Vec::new()),
        },
        None,
    )
}

fn installed(schema: &str) -> bool {
    gio::SettingsSchemaSource::default()
        .and_then(|s| s.lookup(schema, true))
        .is_some()
}

#[test]
fn sources_and_options_write_through_their_service_rows() {
    if !installed(sources::SCHEMA) {
        eprintln!("skipped: no gsettings-desktop-schemas");
        return;
    }
    let mut env = memory();
    let lang = Lang::Nl;
    let args = json!({"sources": [["xkb", "us+intl"], ["xkb", "nl"]]});
    // a dry run validates and writes nothing
    let (_, sync) = sources::set_sources(&mut env, &args, lang, true).unwrap();
    assert_eq!(sync, None);
    let value = |env: &mut Env, key: &str| {
        gsettings::read_item_in(env, sources::SCHEMA, None, key, lang).unwrap()["value"].clone()
    };
    assert_eq!(value(&mut env, "sources"), json!([]));
    let (items, sync) = sources::set_sources(&mut env, &args, lang, false).unwrap();
    assert_eq!(
        items["items"][0]["value"],
        json!([["xkb", "us+intl"], ["xkb", "nl"]])
    );
    assert_eq!(sync, Some(vec!["us,nl".into(), "intl,".into()]));
    // gsettings.set may not write it: amendment 50
    assert_eq!(
        allow::find(sources::SCHEMA, None, "sources").unwrap().mode,
        Mode::Service("input.sources.set")
    );
    let out =
        sources::set_options(&mut env, &json!({"compose": "compose:rctrl"}), lang, false).unwrap();
    assert_eq!(out["options"], json!(["compose:rctrl"]));
    sources::set_options(&mut env, &json!({"lv3": "lv3:menu_switch"}), lang, false).unwrap();
    assert_eq!(
        value(&mut env, "xkb-options"),
        json!(["compose:rctrl", "lv3:menu_switch"])
    );
    sources::set_options(&mut env, &json!({"lv3": null}), lang, true).unwrap();
    assert_eq!(
        value(&mut env, "xkb-options"),
        json!(["compose:rctrl", "lv3:menu_switch"]),
        "dry run"
    );
    sources::set_options(
        &mut env,
        &json!({"lv3": null, "compose": null}),
        lang,
        false,
    )
    .unwrap();
    assert_eq!(value(&mut env, "xkb-options"), json!([]));
    for bad in [
        json!({"lv3": "grp:alt_shift_toggle"}),
        json!({"compose": 3}),
        json!({"compose": "lv3:ralt_alt"}),
    ] {
        assert_eq!(
            sources::set_options(&mut env, &bad, lang, false)
                .unwrap_err()
                .key
                .as_deref(),
            Some("input.badOption"),
            "{bad}"
        );
    }
    // and the sources from the next set's sync argv carry the options there are then
    sources::set_options(&mut env, &json!({"compose": "compose:caps"}), lang, false).unwrap();
    let (_, sync) = sources::set_sources(
        &mut env,
        &json!({"sources": [["xkb", "de+nodeadkeys"]]}),
        lang,
        false,
    )
    .unwrap();
    assert_eq!(
        sync,
        Some(vec![
            "de".into(),
            "nodeadkeys".into(),
            "compose:caps".into()
        ])
    );
}

/// A private session bus only: tests/rust-sandbox/run-tests.sh's (never the live one under /run/user).
fn private_bus_address() -> Option<String> {
    let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").ok()?;
    (std::env::var("GSETTINGS_BACKEND").as_deref() == Ok("keyfile")
        && !address.contains("/run/user/"))
    .then_some(address)
}

type Handler = Box<dyn Fn(&str, &glib::Variant) -> glib::Variant + Send + Sync>;

/// An object on the private session bus in a thread of its own; (its unique name, a stop function).
fn fake_object(
    address: String,
    path: &'static str,
    xml: &'static str,
    iface: &'static str,
    handler: Handler,
) -> (String, Box<dyn FnOnce()>) {
    use std::sync::{atomic::AtomicBool, atomic::Ordering, mpsc};
    let stop = Arc::new(AtomicBool::new(false));
    let (ready, started) = mpsc::channel();
    let thread_stop = stop.clone();
    let server = std::thread::spawn(move || {
        let context = glib::MainContext::new();
        let _ = context.with_thread_default(|| {
            let connection = gio::DBusConnection::for_address_sync(
                &address,
                gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                    | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
                None,
                gio::Cancellable::NONE,
            )
            .unwrap();
            let node = gio::DBusNodeInfo::for_xml(xml).unwrap();
            let info = node.lookup_interface(iface).unwrap();
            let id = connection
                .register_object(
                    path,
                    &info,
                    move |_, _, _, _, method, params, invocation| {
                        let interactive = invocation
                            .message()
                            .flags()
                            .contains(gio::DBusMessageFlags::ALLOW_INTERACTIVE_AUTHORIZATION);
                        assert!(!interactive, "{method}: no password dialog");
                        invocation.return_value(Some(&handler(method, &params)));
                    },
                    |_, _, _, _, _| ().to_variant(),
                    |_, _, _, _, _, _| false,
                )
                .unwrap();
            ready
                .send((
                    connection.unique_name().unwrap().to_string(),
                    context.clone(),
                ))
                .unwrap();
            while !thread_stop.load(Ordering::Acquire) {
                context.iteration(true);
            }
            connection.unregister_object(id).unwrap();
        });
    });
    let (name, context) = started.recv().unwrap();
    (
        name,
        Box::new(move || {
            stop.store(true, Ordering::Release);
            context.wakeup();
            server.join().unwrap();
        }),
    )
}

#[test]
fn the_system_keyboard_follows_only_when_polkit_allows_it_without_asking() {
    let Some(address) = private_bus_address() else {
        eprintln!("skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)");
        return;
    };
    const XML: &str = r#"<node><interface name="org.freedesktop.PolicyKit1.Authority">
        <method name="CheckAuthorization"><arg type="(sa{sv})" direction="in"/><arg type="s" direction="in"/>
        <arg type="a{ss}" direction="in"/><arg type="u" direction="in"/><arg type="s" direction="in"/>
        <arg type="(bba{ss})" direction="out"/></method></interface></node>"#;
    let calls: Arc<Mutex<Vec<(String, String, u32, u32)>>> = Arc::new(Mutex::new(Vec::new()));
    let seen = calls.clone();
    let (name, stop) = fake_object(
        address,
        "/org/freedesktop/PolicyKit1/Authority",
        XML,
        "org.freedesktop.PolicyKit1.Authority",
        Box::new(move |_, params| {
            let subject = params.child_value(0);
            let kind = subject.child_value(0).str().unwrap_or("").to_owned();
            let pid = glib::VariantDict::new(Some(&subject.child_value(1)))
                .lookup_value("pid", None)
                .and_then(|v| v.get::<u32>())
                .unwrap_or(0);
            let action = params.child_value(1).str().unwrap_or("").to_owned();
            let flags = params.child_value(3).get::<u32>().unwrap_or(99);
            seen.lock()
                .unwrap()
                .push((kind, action.clone(), pid, flags));
            let details: std::collections::HashMap<String, String> = Default::default();
            // YES for the keyboard (Ubuntu's rule for sudo users), a challenge for anything else
            let yes = action == sources::KEYBOARD_ACTION;
            ((yes, !yes, details),).to_variant()
        }),
    );
    let bus = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).unwrap();
    assert!(sources::allowed_without_prompt(
        &bus,
        &name,
        sources::KEYBOARD_ACTION
    ));
    assert!(
        !sources::allowed_without_prompt(&bus, &name, "org.freedesktop.locale1.set-keyboard"),
        "a challenge is no"
    );
    assert!(
        !sources::allowed_without_prompt(&bus, ":1.9999", sources::KEYBOARD_ACTION),
        "no polkit: no"
    );
    let got = calls.lock().unwrap().clone();
    assert_eq!(got.len(), 2);
    assert_eq!(
        got[0],
        (
            "unix-process".into(),
            sources::KEYBOARD_ACTION.into(),
            std::process::id(),
            0
        ),
        "this process, flags 0: never a prompt"
    );
    stop();
}

#[test]
fn gnomes_helper_runs_through_pkexec_with_its_argv() {
    let root = temp("pkexec");
    let log = root.join("argv.log");
    // /bin/sh in pkexec's place runs the "helper" (a script that logs its argv): no freshly written file is
    // executed, so no ETXTBSY while other test threads fork
    let script = root.join("helper.sh");
    std::fs::write(
        &script,
        format!(
            "printf '%s\\n' \"$0\" \"$@\" > '{}'\n[ \"$1\" != fail ]\n",
            log.display()
        ),
    )
    .unwrap();
    let sh = Path::new("/bin/sh");
    assert!(sources::run_helper(
        sh,
        &script,
        &["us,nl".into(), "intl,".into()]
    ));
    assert_eq!(
        std::fs::read_to_string(&log).unwrap(),
        format!("{}\nus,nl\nintl,\n", script.display())
    );
    assert!(
        !sources::run_helper(sh, &script, &["fail".into(), String::new()]),
        "the helper's exit code"
    );
    assert!(!sources::run_helper(&root.join("missing"), &script, &[]));
    assert_eq!(
        crate::tools::libexec(sources::HELPER).unwrap(),
        Path::new("/usr/libexec/gnome-control-center-set-debian-keyboard"),
        "GNOME's helper, only ever an argument of pkexec (§5.8)"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn engine_setup_goes_through_gnome_shell() {
    let calls: Mutex<Vec<(String, Value)>> = Mutex::new(Vec::new());
    let ok = |m: &str, a: &Value| -> Result<ApiResult, String> {
        calls.lock().unwrap().push((m.to_owned(), a.clone()));
        Ok(Ok(json!({"launched": true})))
    };
    assert_eq!(
        sources::engine_setup(&json!({"id": "anthy"}), Lang::En, &ok).unwrap(),
        json!({"launched": true})
    );
    assert_eq!(
        *calls.lock().unwrap(),
        vec![("launch.ibusSetup".to_owned(), json!({"engine": "anthy"}))]
    );
    assert_eq!(
        sources::engine_setup(&json!({"id": "a b"}), Lang::En, &ok)
            .unwrap_err()
            .key
            .as_deref(),
        Some("input.badEngine")
    );
    let gone = |_: &str, _: &Value| -> Result<ApiResult, String> { Err("no bus".into()) };
    assert_eq!(
        sources::engine_setup(&json!({"id": "anthy"}), Lang::En, &gone)
            .unwrap_err()
            .code,
        "unsupported"
    );
    let missing = |_: &str, _: &Value| -> Result<ApiResult, String> {
        Ok(Err(ApiError::new("not_found", "no setup")))
    };
    assert_eq!(
        sources::engine_setup(&json!({"id": "anthy"}), Lang::En, &missing)
            .unwrap_err()
            .code,
        "not_found"
    );
}

#[test]
fn the_input_indicator_asks_gnome_shell() {
    let calls: Mutex<Vec<(String, Value)>> = Mutex::new(Vec::new());
    let shell = |m: &str, a: &Value| -> Result<ApiResult, String> {
        calls.lock().unwrap().push((m.to_owned(), a.clone()));
        Ok(Ok(json!({"sources": [], "current": -1})))
    };
    assert_eq!(sources::current(Lang::En, &shell).unwrap()["current"], -1);
    sources::activate(&json!({"index": 1}), Lang::En, &shell).unwrap();
    assert_eq!(
        calls.lock().unwrap()[1],
        ("input.sources.activate".to_owned(), json!({"index": 1}))
    );
    for bad in [
        json!({}),
        json!({"index": -1}),
        json!({"index": 64}),
        json!({"index": "1"}),
    ] {
        assert_eq!(
            sources::activate(&bad, Lang::En, &shell).unwrap_err().code,
            "bad_args",
            "{bad}"
        );
    }
    let gone = |_: &str, _: &Value| -> Result<ApiResult, String> { Err("no bus".into()) };
    assert_eq!(
        sources::current(Lang::En, &gone)
            .unwrap_err()
            .key
            .as_deref(),
        Some("input.shellUnavailable")
    );
}

#[test]
fn keysyms_become_gtk_accelerator_names() {
    assert_eq!(
        capture::accel_name("t", &["Control".into(), "Alt".into()]),
        "<Control><Alt>t"
    );
    assert_eq!(
        capture::accel_name(
            "Page_Up",
            &["Super".into(), "Shift".into(), "Nonsense".into()]
        ),
        "<Shift><Super>Page_Up"
    );
    assert_eq!(capture::timeout_ms(&json!({})), 30_000);
    assert_eq!(capture::timeout_ms(&json!({"timeout": 10})), 1_000);
    assert_eq!(capture::timeout_ms(&json!({"timeout": 999_999})), 120_000);
    let names = |k: u32| match k {
        0x61 => Some("a".to_owned()),
        _ => None,
    };
    assert_eq!(
        capture::answer(&json!({"keyval": 0x61, "mods": ["Super"]}), names),
        Some(json!({"accel": "<Super>a"}))
    );
    assert_eq!(
        capture::answer(&json!({"cancelled": true}), names),
        Some(json!({"cancelled": true}))
    );
    assert_eq!(
        capture::answer(&json!({"disabled": true}), names),
        Some(json!({"disabled": true}))
    );
    assert_eq!(
        capture::answer(&json!({}), names),
        None,
        "the promise of a bridge that cannot wait"
    );
    assert_eq!(
        capture::answer(&json!({"keyval": 0x62, "mods": []}), names),
        None,
        "a keysym without a name"
    );
    assert_eq!(
        capture::answer(&json!({"keyval": -1, "mods": []}), names),
        None
    );
    if !crate::dlib::available("libxkbcommon.so.0") {
        eprintln!("skipped: no libxkbcommon");
        return;
    }
    assert_eq!(
        capture::keysym_name(0x41).as_deref(),
        Some("a"),
        "A is written a, as gtk_accelerator_name does"
    );
    assert_eq!(
        capture::keysym_name(0xff55).as_deref(),
        Some("Page_Up"),
        "GTK's name, not xkb's first (Prior)"
    );
    assert_eq!(capture::keysym_name(0xff56).as_deref(), Some("Page_Down"));
    assert_eq!(capture::keysym_name(0xff09).as_deref(), Some("Tab"));
    assert_eq!(
        capture::keysym_name(0x1008ff12).as_deref(),
        Some("XF86AudioMute")
    );
    assert_eq!(capture::keysym_name(0x3d).as_deref(), Some("equal"));
    assert_eq!(
        capture::keysym_name(0x0eadbeef),
        None,
        "no name: no accelerator"
    );
}

#[test]
fn a_capture_waits_longer_than_its_timeout_and_releases_a_bridge_that_cannot_wait() {
    let calls: Mutex<Vec<(String, Value, i32)>> = Mutex::new(Vec::new());
    let answer = Mutex::new(json!({"keyval": 0x74, "mods": ["Control", "Alt"]}));
    let bridge = |m: &str, a: &Value, ms: i32| -> Result<ApiResult, String> {
        calls.lock().unwrap().push((m.to_owned(), a.clone(), ms));
        Ok(Ok(if m == "input.capture" {
            answer.lock().unwrap().clone()
        } else {
            json!({})
        }))
    };
    let lang = Lang::En;
    let got = capture::capture(&json!({"timeout": 20000}), lang, &bridge);
    if crate::dlib::available("libxkbcommon.so.0") {
        assert_eq!(got.unwrap(), json!({"accel": "<Control><Alt>t"}));
    }
    assert_eq!(
        calls.lock().unwrap()[0],
        ("input.capture".into(), json!({"timeout": 20000}), 25000),
        "5 s longer than the capture"
    );
    calls.lock().unwrap().clear();
    *answer.lock().unwrap() = json!({});
    assert_eq!(
        capture::capture(&json!({}), lang, &bridge)
            .unwrap_err()
            .code,
        "unsupported"
    );
    let seen: Vec<String> = calls.lock().unwrap().iter().map(|c| c.0.clone()).collect();
    assert_eq!(
        seen,
        ["input.capture", "input.capture.cancel"],
        "the grab is released"
    );
    *answer.lock().unwrap() = json!({"cancelled": true});
    assert_eq!(
        capture::capture(&json!({}), lang, &bridge).unwrap(),
        json!({"cancelled": true})
    );
    let refused = |_: &str, _: &Value, _: i32| -> Result<ApiResult, String> {
        Ok(Err(ApiError::new("unknown_method", "x")))
    };
    assert_eq!(
        capture::capture(&json!({}), lang, &refused)
            .unwrap_err()
            .code,
        "unknown_method",
        "KeyCapture listens itself"
    );
    let gone = |_: &str, _: &Value, _: i32| -> Result<ApiResult, String> { Err("no bus".into()) };
    assert_eq!(
        capture::capture(&json!({}), lang, &gone)
            .unwrap_err()
            .key
            .as_deref(),
        Some("input.shellUnavailable")
    );
    assert_eq!(
        capture::cancel(&gone).unwrap(),
        json!({}),
        "nothing to cancel is fine"
    );
}

#[test]
fn accelerators_compare_as_gnome_compares_them() {
    assert_eq!(
        parse_accel("<Primary><Alt>Delete"),
        parse_accel("<Control><alt>delete")
    );
    assert_eq!(parse_accel("<Mod4>l"), parse_accel("<Super>L"));
    assert_eq!(
        parse_accel("<Super>Prior"),
        parse_accel("<Super>Page_Up"),
        "xkb's and GTK's names of one keysym"
    );
    assert_ne!(parse_accel("<Super>l"), parse_accel("<Super><Shift>l"));
    for bad in ["", "<Super>", "<Nope>a", "<Super>a b", "<Super", "a;b"] {
        assert_eq!(parse_accel(bad), None, "{bad:?}");
    }
    assert!(parse_accel("XF86AudioMute").is_some() && parse_accel("<Shift>Print").is_some());
    assert_eq!(
        shortcuts::custom_id(
            "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom12/"
        ),
        Some("custom12")
    );
    assert_eq!(
        shortcuts::custom_id(
            "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/other/"
        ),
        None
    );
    let paths = ["custom0", "custom2"].map(|id| format!("{}{id}/", shortcuts::CUSTOM_DIR));
    assert_eq!(
        shortcuts::free_custom_id(&paths).as_deref(),
        Some("custom1"),
        "the first free one, as GNOME's"
    );
}

#[test]
fn the_builtin_catalogue_is_gnome_settings_own() {
    let builtin = shortcuts::catalogue(Path::new("/nonexistent-root"));
    assert_eq!(
        builtin.len(),
        shortcuts::BUILTIN.len() + shortcuts::WINPOINT.len()
    );
    assert_eq!(
        builtin
            .iter()
            .filter(|e| !e.hidden && e.schema != EXT)
            .count(),
        87
    );
    assert!(
        builtin
            .iter()
            .all(|e| shortcuts::SECTIONS.iter().any(|(id, _)| *id == e.section))
    );
    let dir = Path::new("/usr/share/gnome-control-center/keybindings");
    if !dir.is_dir() {
        eprintln!("skipped: GNOME Settings' keybinding files are not installed");
        return;
    }
    let installed = shortcuts::catalogue(Path::new("/"));
    let key = |e: &shortcuts::Entry| (e.section.clone(), e.schema.clone(), e.key.clone(), e.hidden);
    let mut a: Vec<_> = installed.iter().map(key).collect();
    let mut b: Vec<_> = builtin.iter().map(key).collect();
    a.sort();
    b.sort();
    assert_eq!(
        a, b,
        "the built-in copy is the installed gnome-control-center's"
    );
    let entries = shortcuts::parse_keylist(
        r#"<KeyListEntries group="system" schema="org.gnome.desktop.wm.keybindings" name="Windows">
        <KeyListEntry name="close" description="Close window"/>
        <KeyListEntry name="toggle-tiled-left" schema="org.gnome.mutter.keybindings" description="View split on left"/>
        <KeyListEntry name="raise" hidden="true" description="Raise"/>
        <KeyListEntry name="bad key!" description="x"/></KeyListEntries>"#,
    );
    assert_eq!(entries.len(), 3);
    assert_eq!(
        entries[1].schema, "org.gnome.mutter.keybindings",
        "an entry's own schema"
    );
    assert!(entries[2].hidden && entries[0].section == "windows");
}

/// The extension's real schema, compiled into a private directory (as B0's held test does).
fn extension_schema() -> PathBuf {
    extension_schema_in(temp("ext-schema"))
}

fn extension_schema_in(dir: PathBuf) -> PathBuf {
    std::fs::create_dir_all(&dir).unwrap();
    let xml = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../extension/schemas/org.gnome.shell.extensions.win11-gnome.gschema.xml");
    std::fs::copy(
        &xml,
        dir.join("org.gnome.shell.extensions.win11-gnome.gschema.xml"),
    )
    .unwrap();
    assert!(
        std::process::Command::new("/usr/bin/glib-compile-schemas")
            .arg(&dir)
            .status()
            .unwrap()
            .success()
    );
    dir
}

fn shortcut_env(ext: &Path) -> Env {
    Env::new(
        &Config {
            backend: Backend::Memory,
            extension_dirs: Some(vec![ext.to_path_buf()]),
        },
        None,
    )
}

fn get(env: &mut Env, schema: &str, path: Option<&str>, key: &str) -> Value {
    gsettings::read_item_in(env, schema, path, key, Lang::En).unwrap()["value"].clone()
}

fn put(env: &mut Env, method: &str, schema: &str, path: Option<&str>, key: &str, value: Value) {
    gsettings::write_values_in(env, method, schema, path, &[(key, value)], Lang::En, false)
        .unwrap();
}

fn find_item<'a>(list: &'a Value, schema: &str, key: &str) -> Option<&'a Value> {
    list["sections"]
        .as_array()?
        .iter()
        .flat_map(|s| s["items"].as_array().into_iter().flatten())
        .find(|i| i["schema"] == schema && i["key"] == key)
}

#[test]
fn shortcuts_list_their_sections_and_what_winpoint_holds() {
    if !installed(WM) || !installed(MEDIA) {
        eprintln!("skipped: no GNOME keybinding schemas");
        return;
    }
    let ext = extension_schema();
    let mut env = shortcut_env(&ext);
    let root = Path::new("/nonexistent-root");
    let v = shortcuts::state(&mut env, root, Lang::En).json();
    let ids: Vec<&str> = v["sections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids[0], "winpoint");
    assert!(ids.contains(&"navigation") && ids.contains(&"windows") && ids.contains(&"typing"));
    assert!(
        find_item(&v, WM, "switch-applications-backward").is_none(),
        "hidden, as in GNOME Settings"
    );
    let tv = find_item(&v, EXT, "task-view").unwrap();
    assert_eq!(tv["accels"], json!(["<Alt>Tab"]));
    assert_eq!(
        tv["editable"], true,
        "amendment 13: WinPoint's own keys are editable"
    );
    // the extension stripped <Alt>Tab from switch-applications and keeps the user's value (here: the default)
    put(
        &mut env,
        "input.shortcuts.set",
        WM,
        None,
        "switch-applications",
        json!(["<Super>Tab"]),
    );
    let record =
        json!({"switch-applications": {"user": null, "owned": ["<Super>Tab"]}}).to_string();
    // task-view-restore is Read for the API; the extension writes it (a test-only writer: the backend directly)
    let schema = env.schema(EXT).unwrap();
    let settings = env
        .settings(&schema, "/org/gnome/shell/extensions/win11-gnome/")
        .unwrap();
    settings.set_string("task-view-restore", &record).unwrap();
    settings
        .set_string(
            "logout-restore",
            &json!({"user": null, "owned": []}).to_string(),
        )
        .unwrap();
    let v = shortcuts::state(&mut env, root, Lang::En).json();
    let item = find_item(&v, WM, "switch-applications").unwrap();
    assert_eq!(
        item["held"],
        json!({"accel": "<Alt>Tab", "setting": "task-view"})
    );
    assert_eq!(item["accels"], json!(["<Super>Tab"]));
    assert_eq!(
        find_item(&v, MEDIA, "logout").unwrap()["held"]["setting"],
        "task-manager"
    );
    assert_eq!(find_item(&v, WM, "close").unwrap()["held"], Value::Null);
    let _ = std::fs::remove_dir_all(ext);
}

#[test]
fn a_shortcut_another_one_has_is_a_conflict_until_it_is_replaced() {
    if !installed(WM) || !installed(MEDIA) {
        eprintln!("skipped: no GNOME keybinding schemas");
        return;
    }
    let ext = extension_schema();
    let mut env = shortcut_env(&ext);
    let root = Path::new("/nonexistent-root");
    let lang = Lang::En;
    // <Super>l is the lock screen's (media-keys screensaver)
    let args = json!({"schema": WM, "key": "close", "accels": ["<Super>L"]});
    let out = shortcuts::set(&mut env, root, &args, lang, false).unwrap();
    assert_eq!(out["conflict"]["key"], "screensaver");
    assert_eq!(out["conflict"]["accel"], "<Super>l");
    assert_eq!(
        get(&mut env, WM, None, "close"),
        json!(["<Alt>F4"]),
        "nothing written"
    );
    let mut replace = args.clone();
    replace["replace"] = json!(true);
    shortcuts::set(&mut env, root, &replace, lang, true).unwrap();
    assert_eq!(
        get(&mut env, MEDIA, None, "screensaver"),
        json!(["<Super>l"]),
        "a dry run takes nothing"
    );
    let out = shortcuts::set(&mut env, root, &replace, lang, false).unwrap();
    assert_eq!(out["replaced"][0]["key"], "screensaver");
    assert_eq!(get(&mut env, WM, None, "close"), json!(["<Super>L"]));
    assert_eq!(get(&mut env, MEDIA, None, "screensaver"), json!([]));
    // WinPoint's Task View holds <Alt>Tab (the extension took it from switch-applications): a conflict with
    // WinPoint's key (amendment 13), Vervangen frees it first
    put(
        &mut env,
        "input.shortcuts.set",
        WM,
        None,
        "switch-applications",
        json!(["<Super>Tab"]),
    );
    let alt_tab = json!({"schema": WM, "key": "switch-windows", "accels": ["<Alt>Tab"]});
    assert_eq!(
        shortcuts::set(&mut env, root, &alt_tab, lang, false).unwrap()["conflict"]["key"],
        "task-view"
    );
    let mut take = alt_tab.clone();
    take["replace"] = json!(true);
    shortcuts::set(&mut env, root, &take, lang, false).unwrap();
    assert_eq!(get(&mut env, EXT, None, "task-view"), json!([]));
    assert_eq!(
        get(&mut env, WM, None, "switch-windows"),
        json!(["<Alt>Tab"])
    );
    // off, reset, the refusals
    shortcuts::set(
        &mut env,
        root,
        &json!({"schema": WM, "key": "close", "accels": []}),
        lang,
        false,
    )
    .unwrap();
    assert_eq!(get(&mut env, WM, None, "close"), json!([]));
    shortcuts::reset(
        &mut env,
        root,
        &json!({"schema": WM, "key": "close"}),
        lang,
        false,
    )
    .unwrap();
    assert_eq!(get(&mut env, WM, None, "close"), json!(["<Alt>F4"]));
    for (bad, key) in [
        (
            json!({"schema": WM, "key": "close", "accels": ["<Nope>x"]}),
            "input.badShortcut",
        ),
        (
            json!({"schema": WM, "key": "close", "accels": ["a", "b", "c", "d", "e"]}),
            "input.badShortcut",
        ),
        (
            json!({"schema": WM, "key": "close", "accels": "<Super>x"}),
            "input.badShortcut",
        ),
        (
            json!({"schema": WM, "key": "close", "path": "/x/", "accels": []}),
            "input.badShortcut",
        ),
        (
            json!({"schema": WM, "key": "audible-bell", "accels": []}),
            "input.noShortcut",
        ),
        (
            json!({"schema": "org.gnome.desktop.interface", "key": "gtk-theme", "accels": []}),
            "input.noShortcut",
        ),
    ] {
        assert_eq!(
            shortcuts::set(&mut env, root, &bad, lang, false)
                .unwrap_err()
                .key
                .as_deref(),
            Some(key),
            "{bad}"
        );
    }
    // Alles opnieuw instellen: GNOME's keys, not WinPoint's
    put(
        &mut env,
        "input.shortcuts.set",
        EXT,
        None,
        "task-manager",
        json!(["<Super>Escape"]),
    );
    let n = shortcuts::reset_all(&mut env, root, lang, false).unwrap()["reset"]
        .as_u64()
        .unwrap();
    assert!(n > 80, "{n}");
    assert_eq!(
        get(&mut env, MEDIA, None, "screensaver"),
        json!(["<Super>l"])
    );
    assert_eq!(
        get(&mut env, EXT, None, "task-manager"),
        json!(["<Super>Escape"]),
        "WinPoint's own keys stay"
    );
    let _ = std::fs::remove_dir_all(ext);
}

#[test]
fn custom_shortcuts_are_added_changed_and_removed_as_gnome_settings_does() {
    if !installed(MEDIA) || !installed(CUSTOM) {
        eprintln!("skipped: no media-keys schemas");
        return;
    }
    let ext = extension_schema();
    let mut env = shortcut_env(&ext);
    let root = Path::new("/nonexistent-root");
    let lang = Lang::En;
    let add = json!({"name": "Terminal here", "command": "kgx", "binding": "<Super>t"});
    shortcuts::custom_add(&mut env, root, &add, lang, true).unwrap();
    assert_eq!(
        get(&mut env, MEDIA, None, "custom-keybindings"),
        json!([]),
        "dry run"
    );
    let out = shortcuts::custom_add(&mut env, root, &add, lang, false).unwrap();
    assert_eq!(out["id"], "custom0");
    let path = out["path"].as_str().unwrap().to_owned();
    assert_eq!(
        get(&mut env, MEDIA, None, "custom-keybindings"),
        json!([path])
    );
    assert_eq!(get(&mut env, CUSTOM, Some(&path), "command"), "kgx");
    // the same binding again: a conflict with the first one
    let second = json!({"name": "Other", "command": "true", "binding": "<Super>T"});
    let out = shortcuts::custom_add(&mut env, root, &second, lang, false).unwrap();
    assert_eq!(
        (
            out["conflict"]["id"].clone(),
            out["conflict"]["name"].clone()
        ),
        (json!("custom0"), json!("Terminal here"))
    );
    let v = shortcuts::state(&mut env, root, lang).json();
    assert_eq!(v["custom"].as_array().unwrap().len(), 1);
    // a custom binding against GNOME's own: <Super>l (lock screen)
    let lock = json!({"id": "custom0", "binding": "<Super>l"});
    assert_eq!(
        shortcuts::custom_update(&mut env, root, &lock, lang, false).unwrap()["conflict"]["key"],
        "screensaver"
    );
    shortcuts::custom_update(
        &mut env,
        root,
        &json!({"id": "custom0", "name": "Console", "binding": ""}),
        lang,
        false,
    )
    .unwrap();
    assert_eq!(get(&mut env, CUSTOM, Some(&path), "name"), "Console");
    assert_eq!(get(&mut env, CUSTOM, Some(&path), "binding"), "");
    for (bad, key) in [
        (
            json!({"name": "", "command": "x", "binding": ""}),
            "input.badName",
        ),
        (
            json!({"name": "a\nb", "command": "x", "binding": ""}),
            "input.badName",
        ),
        (
            json!({"name": "a", "command": " ", "binding": ""}),
            "input.badCommand",
        ),
        (
            json!({"name": "a", "command": "x", "binding": "<Super"}),
            "input.badShortcut",
        ),
    ] {
        assert_eq!(
            shortcuts::custom_add(&mut env, root, &bad, lang, false)
                .unwrap_err()
                .key
                .as_deref(),
            Some(key),
            "{bad}"
        );
    }
    assert_eq!(
        shortcuts::custom_remove(&mut env, root, &json!({"id": "custom7"}), lang, false)
            .unwrap_err()
            .key
            .as_deref(),
        Some("input.noShortcut")
    );
    shortcuts::custom_remove(&mut env, root, &json!({"id": "custom0"}), lang, false).unwrap();
    assert_eq!(get(&mut env, MEDIA, None, "custom-keybindings"), json!([]));
    let name = gsettings::read_item_in(&mut env, CUSTOM, Some(&path), "name", lang).unwrap();
    assert_eq!(name["isDefault"], true, "its keys are reset");
    let _ = std::fs::remove_dir_all(ext);
}

/// The keys the pages Typen, Taalopties, Geavanceerde toetsenbordinstellingen and Sneltoetsen read or write
/// (native/apps/settings/src/pages/{typing,keyboards,shortcuts}.rs), with the mode each needs.
#[test]
fn every_key_the_keyboard_pages_bind_has_its_allow_row() {
    let write = [
        (
            "org.gnome.desktop.a11y.applications",
            "screen-keyboard-enabled",
        ),
        ("org.gnome.desktop.input-sources", "per-window"),
        ("org.gnome.desktop.peripherals.keyboard", "repeat"),
        ("org.gnome.desktop.peripherals.keyboard", "delay"),
        ("org.gnome.desktop.peripherals.keyboard", "repeat-interval"),
        ("org.gnome.mutter", "overlay-key"),
    ];
    for (schema, key) in write {
        assert_eq!(
            allow::find(schema, None, key).map(|r| r.mode),
            Some(Mode::Write),
            "{schema} {key}"
        );
    }
    for (schema, key, method) in [
        (sources::SCHEMA, "sources", "input.sources.set"),
        (sources::SCHEMA, "xkb-options", "input.options.set"),
        (WM, "switch-input-source", "input.shortcuts.set"),
        (MEDIA, "custom-keybindings", "input.shortcuts.set"),
        (EXT, "task-view", "input.shortcuts.set"),
        (EXT, "task-manager", "input.shortcuts.set"),
    ] {
        assert_eq!(
            allow::find(schema, None, key).map(|r| r.mode),
            Some(Mode::Service(method)),
            "{schema} {key}"
        );
    }
    let path = format!("{}custom3/", shortcuts::CUSTOM_DIR);
    assert_eq!(
        allow::find(CUSTOM, Some(&path), "binding").map(|r| r.mode),
        Some(Mode::Service("input.shortcuts.custom.update"))
    );
    assert!(
        allow::find(
            CUSTOM,
            Some("/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/x/"),
            "binding"
        )
        .is_none()
    );
    for key in ["task-view-restore", "logout-restore"] {
        assert_eq!(
            allow::find(EXT, None, key).map(|r| r.mode),
            Some(Mode::Read),
            "the extension's records are read only"
        );
    }
    // the methods named by a row exist in METHODS (a row for a method nobody implements would be dead)
    for (_, row) in allow::rows() {
        if let Mode::Service(m) = row.mode
            && m.starts_with("input.")
        {
            assert!(METHODS.iter().any(|(name, _)| *name == m), "{m}");
        }
    }
}

fn keyfile_service(tag: &str, dry_run: bool) -> (Input, Arc<crate::events::Events>, PathBuf) {
    let root = temp(tag);
    let events = crate::events::Events::new(&root.join("events")).unwrap();
    let ctx = ServiceContext::new(
        events.clone(),
        root.join("config"),
        root.join("cache"),
        root.clone(),
        "settings".into(),
        dry_run,
    )
    .unwrap();
    std::fs::create_dir_all(root.join("defaults")).unwrap();
    let ext = extension_schema_in(root.join("ext"));
    let gs = Config {
        backend: Backend::Keyfile {
            file: root.join("keyfile"),
            defaults: root.join("defaults"),
        },
        extension_dirs: Some(vec![ext]),
    };
    (
        Input {
            ctx,
            root: root.clone(),
            gs,
        },
        events,
        root,
    )
}

#[test]
fn the_dispatch_writes_announces_and_keeps_a_dry_run_dry() {
    if !installed(sources::SCHEMA) || !installed(WM) {
        eprintln!("skipped: no GNOME schemas");
        return;
    }
    let no_bridge = |_: &str, _: &Value| -> Result<ApiResult, String> { Err("no bus".into()) };
    let no_timed =
        |_: &str, _: &Value, _: i32| -> Result<ApiResult, String> { Err("no bus".into()) };
    for dry_run in [true, false] {
        let (input, events, root) = keyfile_service(if dry_run { "dry" } else { "wet" }, dry_run);
        let seen: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        events.on(
            "input.changed",
            Arc::new(move |d| s2.lock().unwrap().push(d.clone())),
        );
        let call = |m: &str, a: Value| input.call_with(m, &a, &no_bridge, &no_timed).unwrap();
        // no xkb source: nothing for the system keyboard (and the sandbox has no polkit anyway)
        let out = call("input.sources.set", json!({"sources": [["ibus", "anthy"]]})).unwrap();
        assert_eq!(out["systemKeyboard"], "skipped");
        call("input.options.set", json!({"compose": "compose:menu"})).unwrap();
        let conflict = call(
            "input.shortcuts.set",
            json!({"schema": WM, "key": "close", "accels": ["<Super>l"]}),
        )
        .unwrap();
        assert!(conflict["conflict"].is_object());
        call(
            "input.shortcuts.set",
            json!({"schema": WM, "key": "close", "accels": ["<Super>q"]}),
        )
        .unwrap();
        let list = call("input.shortcuts", json!({})).unwrap();
        let close = find_item(&list, WM, "close").unwrap();
        let whats: Vec<Value> = seen
            .lock()
            .unwrap()
            .iter()
            .map(|d| d["what"].clone())
            .collect();
        if dry_run {
            assert_eq!(
                close["accels"],
                json!(["<Alt>F4"]),
                "a dry run writes nothing"
            );
            assert!(whats.is_empty(), "and announces nothing: {whats:?}");
        } else {
            assert_eq!(close["accels"], json!(["<Super>q"]));
            assert_eq!(
                whats,
                [json!("sources"), json!("options"), json!("shortcuts")],
                "no event for the conflict"
            );
        }
        assert_eq!(
            call("input.capture", json!({})).unwrap_err().code,
            "unsupported"
        );
        assert_eq!(call("input.capture.cancel", json!({})).unwrap(), json!({}));
        assert!(call("input.preview", json!({"layout": "us intl"})).is_err());
        events.shutdown();
        let _ = std::fs::remove_dir_all(root);
    }
}

#[test]
fn a_write_of_another_service_reaches_gsettings_changed() {
    // the Settings pages follow gsettings.changed, also after input.sources.set and input.options.set: a write through
    // write_values (another service's Env, another thread, the default backend) reaches the gsettings worker's watch
    if std::env::var("GSETTINGS_BACKEND").as_deref() != Ok("keyfile") || !installed(sources::SCHEMA)
    {
        eprintln!("skipped: not in tests/rust-sandbox/run-tests.sh");
        return;
    }
    let root = temp("changed");
    let events = crate::events::Events::new(&root.join("events")).unwrap();
    let ctx = ServiceContext::new(
        events.clone(),
        root.join("config"),
        root.join("cache"),
        root.clone(),
        "settings".into(),
        false,
    )
    .unwrap();
    let seen: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let s2 = seen.clone();
    events.on(
        "gsettings.changed",
        Arc::new(move |d| s2.lock().unwrap().push(d.clone())),
    );
    let service = gsettings::GSettings::new(ctx).unwrap();
    let key = json!({"keys": [{"schema": sources::SCHEMA, "key": "per-window"}]});
    let before = service.call("gsettings.get", &key).unwrap().unwrap()["items"][0]["value"] == true;
    gsettings::write_values(
        "input.sources.set",
        sources::SCHEMA,
        None,
        &[("per-window", json!(!before))],
        Lang::En,
        false,
    )
    .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut heard = false;
    while std::time::Instant::now() < deadline && !heard {
        std::thread::sleep(std::time::Duration::from_millis(50));
        heard = seen
            .lock()
            .unwrap()
            .iter()
            .any(|d| d["key"] == "per-window");
    }
    eprintln!("gsettings.changed after another service's write: {heard}");
    // what the worker reads now: the write itself is there in any case
    let after = service.call("gsettings.get", &key).unwrap().unwrap()["items"][0]["value"].clone();
    assert_eq!(after, json!(!before));
    gsettings::write_values(
        "input.sources.set",
        sources::SCHEMA,
        None,
        &[("per-window", json!(before))],
        Lang::En,
        false,
    )
    .unwrap();
    service.shutdown();
    events.shutdown();
    let _ = std::fs::remove_dir_all(root);
    assert!(
        heard,
        "a write through write_values is heard by the gsettings worker"
    );
}

#[test]
fn reset_all_preserves_shortcuts_with_defaults_held_by_winpoint() {
    if !installed(WM) || !installed(MEDIA) {
        return;
    }
    let ext = extension_schema();
    let mut env = shortcut_env(&ext);
    let root = Path::new("/nonexistent-root");
    let lang = Lang::En;
    put(
        &mut env,
        "input.shortcuts.set",
        WM,
        None,
        "switch-applications",
        json!(["<Super>q"]),
    );
    put(
        &mut env,
        "input.shortcuts.set",
        WM,
        None,
        "close",
        json!(["<Super>z"]),
    );
    let args = json!({ "schema": WM, "key": "switch-applications" });
    for dry in [true, false] {
        let error = shortcuts::reset(&mut env, root, &args, lang, dry).unwrap_err();
        assert_eq!(error.key.as_deref(), Some("gsettings.heldByWinPoint"));
        shortcuts::reset_all(&mut env, root, lang, dry).unwrap();
        assert_eq!(
            get(&mut env, WM, None, "switch-applications"),
            json!(["<Super>q"])
        );
        assert_eq!(get(&mut env, EXT, None, "task-view"), json!(["<Alt>Tab"]));
        assert_eq!(
            get(&mut env, WM, None, "close"),
            if dry {
                json!(["<Super>z"])
            } else {
                json!(["<Alt>F4"])
            }
        );
    }
    drop(env);
    std::fs::remove_dir_all(ext).unwrap();
}
