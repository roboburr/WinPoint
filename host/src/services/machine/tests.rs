//! Tests of `machine.*`: the pure parsers and mappings on fixtures (or on the installed files, skipped when they are
//! not there), and the hostname1, timedate1, locale1, GNOME Shell, systemd and PackageKit calls against fakes on
//! tests/rust-sandbox/run-tests.sh's private session bus (users/tests.rs's FakeBus: every call recorded with whether
//! it allowed interactive authorization). The system-bus run is `fakes_machine_on_the_system_bus`
//! (tests/dbus-fakes/run.sh --batch e, fakes/e/machine.py).
use super::*;
use crate::services::system::common::LangContext;
use crate::services::users::tests::{FakeBus, Methods, Props, context, fake_calls};
use std::path::Path;
use std::time::Duration;

// ------------------------------------------------------------------------------------------------ fixtures

/// A temporary root for Buses::root, removed when dropped.
struct Root(PathBuf);

impl Root {
    fn new() -> Root {
        let dir = std::env::temp_dir().join(format!("win11-machine-{}", crate::events::nonce()));
        std::fs::create_dir_all(&dir).unwrap();
        Root(dir)
    }

    fn write(&self, path: &str, text: &str) -> &Root {
        let file = self.0.join(path.trim_start_matches('/'));
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
        self
    }

    fn buses(&self) -> Buses {
        Buses {
            root: Some(self.0.clone()),
            ..Buses::default()
        }
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const LANGUAGELIST: &str = "#\n# This is the complete list of languages (locales) to choose from.\n\
C;0;C;C;en;;\n\
Dutch;1;nl;nl_NL.UTF-8;nl;NL;nl:en_GB:en;kbd=lat0-sun16(utf8)\n\
English (UK);0;en;en_GB.UTF-8;en;GB;en_GB:en;kbd=lat0-sun16(utf8)\n\
English (USA);0;en;en_US.UTF-8;en;US;en_US:en;kbd=lat0-sun16(utf8)\n\
Chinese (China);2;zh;zh_CN.UTF-8;zh;CN;zh_CN:zh:en_US:en;\n\
Chinese (Taiwan);2;zh;zh_TW.UTF-8;zh;TW;zh_TW:zh:en_US:en;\n\
German;1;de;de_DE.UTF-8;de;DE;de_DE:de:en_GB:en;kbd=lat0-sun16(utf8)\n\
Dzongkha;4;dz;BT;dz_BT;UTF-8;dz_BT;\n";
const LANGPACKS: &str = "zh_CN:zh-hans\nzh_HK:zh-hant\nzh_TW:zh-hant\n";
const PKG_DEPENDS: &str = "# Format: %LCODE%\ntr:::language-pack-\ntr::gvfs:language-pack-gnome-\n\
tr::libreoffice-common:libreoffice-l10n-\nwa:::hunspell-\nfn:::fonts-noto-core\n\
wa:nl::wdutch\nwa:de::wngerman\nfn:zh-hans::fonts-noto-cjk\n";
const DPKG: &str = "Package: gvfs\nStatus: install ok installed\nVersion: 1\n\n\
Package: language-pack-de\nStatus: install ok installed\n\n\
Package: language-pack-de-base\nStatus: install ok installed\n\n\
Package: language-pack-gnome-de\nStatus: install ok installed\n\n\
Package: wngerman\nStatus: deinstall ok config-files\n\n\
Package: language-pack-en\nStatus: install ok installed\n\n\
Package: dpkg\nStatus: install ok installed\nArchitecture: amd64\nMulti-Arch: foreign\n";

fn language_root() -> Root {
    let r = Root::new();
    r.write(
        "/usr/share/language-selector/data/languagelist",
        LANGUAGELIST,
    )
    .write(
        "/usr/share/language-selector/data/locale2langpack",
        LANGPACKS,
    )
    .write("/usr/share/language-selector/data/pkg_depends", PKG_DEPENDS)
    .write("/var/lib/dpkg/status", DPKG)
    .write("/usr/share/zoneinfo/Europe/Amsterdam", "TZif")
    .write(
        "/usr/share/dbus-1/system-services/org.freedesktop.PackageKit.service",
        "[D-BUS Service]\n",
    );
    r
}

// ------------------------------------------------------------------------------------------------ pure parts

#[test]
fn the_static_hostname_is_derived_as_gnome_derives_it() {
    assert_eq!(
        hostname::static_name("Robins Laptop").as_deref(),
        Some("robins-laptop")
    );
    assert_eq!(
        hostname::static_name("Émile's PC").as_deref(),
        Some("emiles-pc")
    );
    assert_eq!(
        hostname::static_name("  --a__b--  ").as_deref(),
        Some("a-b")
    );
    assert_eq!(
        hostname::static_name("Straße 7").as_deref(),
        Some("strasse-7")
    );
    assert_eq!(hostname::static_name("!!!"), None);
    assert_eq!(hostname::static_name("日本"), None, "nothing ASCII is left");
    let long = hostname::static_name(&"a".repeat(80)).unwrap();
    assert_eq!(long.len(), 63);
    assert_eq!(
        hostname::clean("  Werkplek  "),
        Some(("Werkplek".to_owned(), "werkplek".to_owned()))
    );
    assert_eq!(hostname::clean(""), None);
    assert_eq!(hostname::clean("a\tb"), None, "no control characters");
    assert_eq!(
        hostname::clean(&"x".repeat(65)),
        None,
        "GNOME's 64 characters"
    );
}

#[test]
fn zones_are_labelled_and_sorted_as_windows_shows_them() {
    let tab = "# tzdb timezone descriptions\n#codes\tcoordinates\tTZ\tcomments\n\
NL,BE\t+5222+00454\tEurope/Brussels\n\
AR\t-3436-05827\tAmerica/Argentina/Buenos_Aires\tBuenos Aires (BA, CF)\n\
IN\t+2232+08822\tAsia/Kolkata\n\
CA\t+4734-05243\tAmerica/St_Johns\tNewfoundland\n\
XX\t+0000+00000\t../etc/passwd\n";
    assert_eq!(
        zones::parse_tab(tab),
        [
            "Europe/Brussels",
            "America/Argentina/Buenos_Aires",
            "Asia/Kolkata",
            "America/St_Johns"
        ]
    );
    assert_eq!(zones::city("Europe/Amsterdam"), "Amsterdam");
    assert_eq!(
        zones::city("America/Argentina/Buenos_Aires"),
        "Buenos Aires (Argentina)"
    );
    assert_eq!(zones::city("UTC"), "UTC");
    assert_eq!(zones::offset_text(60), "(UTC+01:00)");
    assert_eq!(zones::offset_text(-210), "(UTC-03:30)");
    assert_eq!(zones::offset_text(0), "(UTC+00:00)");
    assert!(
        !zones::valid_name("../etc/passwd")
            && !zones::valid_name("/etc/x")
            && !zones::valid_name("")
    );
    assert!(zones::valid_name("America/Port-au-Prince") && zones::valid_name("Etc/GMT+5"));
    assert_eq!(
        zones::label("UTC", Lang::En),
        "(UTC) Coordinated Universal Time"
    );
    assert_eq!(
        zones::label("Etc/UTC", Lang::Nl),
        "(UTC) Gecoördineerde wereldtijd"
    );
    if glib::TimeZone::from_identifier(Some("Europe/Amsterdam")).is_some()
        && Path::new("/usr/share/zoneinfo/Europe/Amsterdam").is_file()
    {
        // the standard offset, also in summer
        assert_eq!(zones::standard_offset("Europe/Amsterdam"), Some(60));
        assert_eq!(zones::standard_offset("Asia/Kolkata"), Some(330));
        assert_eq!(zones::standard_offset("America/St_Johns"), Some(-210));
        assert_eq!(
            zones::standard_offset("Europe/Dublin"),
            Some(0),
            "Dublin's winter time"
        );
        assert_eq!(
            zones::label("Europe/Amsterdam", Lang::Nl),
            "(UTC+01:00) Amsterdam"
        );
        let root = Root::new();
        root.write("/usr/share/zoneinfo/zone1970.tab", tab);
        let list = zones::list(&root.buses(), Some("Europe/Amsterdam"), Lang::En);
        let ids: Vec<&str> = list["zones"]
            .as_array()
            .unwrap()
            .iter()
            .map(|z| z["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            [
                "America/St_Johns",
                "America/Argentina/Buenos_Aires",
                "UTC",
                "Europe/Amsterdam",
                "Europe/Brussels",
                "Asia/Kolkata"
            ],
            "by offset, then city; UTC and the current zone added"
        );
        assert_eq!(list["zones"][5]["label"], "(UTC+05:30) Kolkata");
        assert_eq!(list["zones"][0]["offset"], -210);
    } else {
        eprintln!("skipped the offsets: no tzdata");
    }
}

#[test]
fn the_time_server_comes_from_chronys_files_and_no_timesync1() {
    let conf = "# chrony.conf\nconfdir /etc/chrony/conf.d\nsourcedir /run/chrony-dhcp\n\
sourcedir /etc/chrony/sources.d\nkeyfile /etc/chrony/chrony.keys\nmakestep 1 3\n";
    let root = Root::new();
    root.write("/etc/chrony/chrony.conf", conf)
        .write(
            "/etc/chrony/sources.d/ubuntu-ntp-pools.sources",
            "pool 1.ntp.ubuntu.com iburst maxsources 1 nts prefer\npool 2.ntp.ubuntu.com iburst\n",
        )
        .write("/etc/chrony/sources.d/README", "server not.a.source\n")
        .write("/etc/chrony/conf.d/local.conf", "# none\n");
    assert_eq!(
        time::server(&root.buses()).as_deref(),
        Some("1.ntp.ubuntu.com")
    );
    let dir = |d: &str, ext: &str| -> Vec<String> {
        match (d, ext) {
            ("/etc/chrony/conf.d", "conf") => vec!["server time.example.org iburst\n".into()],
            ("/etc/chrony/sources.d", "sources") => {
                vec!["pool time.example.org\npool b.example.org\n".into()]
            }
            _ => Vec::new(),
        }
    };
    assert_eq!(
        time::chrony_servers(&format!("server a.example.org\n{conf}"), &dir),
        ["a.example.org", "time.example.org", "b.example.org"]
    );
    assert_eq!(
        time::timesyncd_servers("[Time]\n#NTP=\nFallbackNTP=ntp.ubuntu.com\n"),
        ["ntp.ubuntu.com"]
    );
    assert_eq!(
        time::timesyncd_servers("[Time]\nNTP=a b\nFallbackNTP=c\n"),
        ["a", "b"]
    );
    // neither file: no server, the page hides the row
    assert_eq!(time::server(&Root::new().buses()), None);
    assert!(
        time::valid_usec(1_763_300_000_000_000)
            && !time::valid_usec(0)
            && !time::valid_usec(i64::MAX)
    );
}

#[test]
fn locales_are_normalised_and_named_as_gnome_desktop_names_them() {
    assert_eq!(
        locale::normalize("nl_NL.utf8").as_deref(),
        Some("nl_NL.UTF-8")
    );
    assert_eq!(
        locale::normalize("sr_RS.utf8@latin").as_deref(),
        Some("sr_RS.UTF-8@latin")
    );
    assert_eq!(locale::normalize("C.utf8"), None);
    assert_eq!(locale::normalize("POSIX"), None);
    assert_eq!(locale::normalize("nl; rm"), None);
    assert_eq!(
        locale::clean_list(
            [
                "en_AG",
                "en_AG.utf8",
                "en_IL",
                "nl_NL.utf8",
                "C.utf8",
                "nl_NL.utf8"
            ]
            .into_iter()
        ),
        ["en_AG.UTF-8", "en_IL", "nl_NL.UTF-8"],
        "a UTF-8 one where there is one, no doubles, no C"
    );
    assert_eq!(
        locale::parts("sr_RS.UTF-8@latin"),
        (
            "sr".to_owned(),
            Some("RS".to_owned()),
            Some("latin".to_owned())
        )
    );
    assert_eq!(locale::first_item("Dutch; Flemish"), "Dutch");
    assert_eq!(locale::first_item("français"), "Français");
    // a locale archive (locarchive.h): header, the name hash table, the names
    let mut archive = vec![0u8; 200];
    let put = |a: &mut Vec<u8>, at: usize, v: u32| a[at..at + 4].copy_from_slice(&v.to_ne_bytes());
    put(&mut archive, 0, 0xde02_0109);
    put(&mut archive, 8, 56); // namehash_offset
    put(&mut archive, 16, 3); // namehash_size
    for (i, (name_at, record)) in [(120u32, 1u32), (0, 0), (140, 7)].into_iter().enumerate() {
        put(&mut archive, 56 + i * 12 + 4, name_at);
        put(&mut archive, 56 + i * 12 + 8, record);
    }
    archive[120..131].copy_from_slice(b"nl_NL.utf8\0");
    archive[140..151].copy_from_slice(b"en_GB.utf8\0");
    assert_eq!(
        locale::archive_names(&archive),
        ["nl_NL.utf8", "en_GB.utf8"]
    );
    assert!(locale::archive_names(b"not an archive at all").is_empty());
    // a gettext catalogue, little endian: one header entry and one string
    let mut mo = Vec::new();
    for v in [0x9504_12deu32, 0, 2, 28, 44, 0, 0] {
        mo.extend_from_slice(&v.to_le_bytes());
    }
    for (len, off) in [(0u32, 60u32), (5, 61)] {
        mo.extend_from_slice(&len.to_le_bytes());
        mo.extend_from_slice(&off.to_le_bytes());
    }
    for (len, off) in [(4u32, 67u32), (10, 72)] {
        mo.extend_from_slice(&len.to_le_bytes());
        mo.extend_from_slice(&off.to_le_bytes());
    }
    mo.resize(60, 0);
    mo.extend_from_slice(b"\0Dutch\0head\0Nederlands\0");
    let catalogue = locale::parse_mo(&mo);
    assert_eq!(
        catalogue.get("Dutch").map(String::as_str),
        Some("Nederlands")
    );
    assert_eq!(catalogue.len(), 1, "the header is no translation");
    // iso-codes' names, when installed
    let b = Buses::default();
    if Path::new("/usr/share/iso-codes/json/iso_639-2.json").is_file() {
        assert_eq!(
            locale::display_name(&b, "nl_NL.UTF-8", "en"),
            "Dutch (Netherlands)"
        );
        assert_eq!(
            locale::display_name(&b, "en_GB.UTF-8", "en"),
            "English (United Kingdom)"
        );
        assert_eq!(
            locale::display_name(&b, "en_US.UTF-8", "en"),
            "English (United States)"
        );
        if Path::new("/usr/share/locale/nl/LC_MESSAGES/iso_639-2.mo").is_file() {
            assert_eq!(
                locale::display_name(&b, "nl_NL.UTF-8", "nl"),
                "Nederlands (Nederland)"
            );
            assert_eq!(
                locale::display_name(&b, "en_US.UTF-8", "nl"),
                "Engels (Verenigde Staten)"
            );
            assert_eq!(
                locale::display_name(&b, "nl_BE.UTF-8", "nl_BE"),
                "Nederlands (België)"
            );
        }
        assert_eq!(
            locale::display_name(&b, "xx_YY.UTF-8", "en"),
            "xx_YY.UTF-8",
            "no name: the id"
        );
    } else {
        eprintln!("skipped the names: no iso-codes");
    }
}

#[test]
fn the_formats_preview_reads_the_locale_itself() {
    assert_eq!(locale::group("123456789", ".", &[3]), "123.456.789");
    assert_eq!(
        locale::group("123456789", ",", &[3, 2]),
        "12,34,56,789",
        "en_IN"
    );
    assert_eq!(
        locale::group("123456789", " ", &[3, 127]),
        "123456 789",
        "one group, then none"
    );
    assert_eq!(locale::group("123456789", "", &[3]), "123456789");
    assert_eq!(locale::group("12", ".", &[3]), "12");
    assert_eq!(locale::paper_name(210, 297), Some("A4"));
    assert_eq!(locale::paper_name(216, 279), Some("US Letter"));
    assert_eq!(locale::paper_name(100, 100), None);
    // 16 November 2026, 14:35:00 UTC, in the generated locales the computer has
    let at = 1_794_839_700;
    if let Some(gb) = locale::preview("en_GB.UTF-8", at) {
        assert_eq!(gb["number"], "123,456,789.00");
        assert_eq!(gb["measurement"], "metric");
        assert_eq!(gb["paper"], "A4");
        assert!(gb["date"].as_str().unwrap().contains("16/11/"), "{gb}");
    }
    if let Some(us) = locale::preview("en_US.UTF-8", at) {
        assert_eq!(us["measurement"], "imperial");
        assert_eq!(us["paper"], "US Letter");
    }
    if let Some(nl) = locale::preview("nl_NL.UTF-8", at) {
        assert_eq!(nl["number"], "123.456.789,00");
    }
    assert!(
        locale::preview("xx_YY.UTF-8", at).is_none(),
        "not generated: no preview"
    );
}

#[test]
fn the_sign_in_screen_gets_gnomes_locale_list() {
    assert_eq!(
        locale::compose("nl_NL.UTF-8", Some("en_GB.UTF-8")),
        [
            "LANG=nl_NL.UTF-8",
            "LC_TIME=en_GB.UTF-8",
            "LC_NUMERIC=en_GB.UTF-8",
            "LC_MONETARY=en_GB.UTF-8",
            "LC_MEASUREMENT=en_GB.UTF-8",
            "LC_PAPER=en_GB.UTF-8"
        ]
    );
    assert_eq!(
        locale::compose("nl_NL.UTF-8", Some("")),
        ["LANG=nl_NL.UTF-8"]
    );
    assert_eq!(locale::compose("nl_NL.UTF-8", None), ["LANG=nl_NL.UTF-8"]);
    let map =
        locale::parse_locale_list(&json!(["LANG=en_US.UTF-8", "LC_TIME=nl_NL.UTF-8", "junk"]));
    assert_eq!(map.get("LANG").map(String::as_str), Some("en_US.UTF-8"));
    assert_eq!(map.get("LC_TIME").map(String::as_str), Some("nl_NL.UTF-8"));
    assert_eq!(map.len(), 2);
}

#[test]
fn language_support_follows_language_selectors_data() {
    let packs = languages::parse_langpacks(LANGPACKS);
    let list = languages::parse_languagelist(LANGUAGELIST, &packs);
    let codes: Vec<&str> = list.iter().map(|e| e.code.as_str()).collect();
    assert_eq!(
        codes,
        ["nl", "en", "zh-hans", "zh-hant", "de", "dz"],
        "C left out, one entry per pack"
    );
    assert_eq!(list[0].locale, "nl_NL.UTF-8");
    assert_eq!(
        list[5].locale, "dz_BT.UTF-8",
        "Ubuntu's Dzongkha line, whose fields are in another order"
    );
    let rules = languages::parse_pkg_depends(PKG_DEPENDS);
    assert_eq!(rules.len(), 8);
    let installed = languages::parse_dpkg_status(DPKG);
    assert!(installed.contains("gvfs") && installed.contains("language-pack-de"));
    assert!(
        !installed.contains("wngerman"),
        "config-files only is not installed"
    );
    assert_eq!(
        languages::wanted(&rules, "nl", &installed),
        [
            "language-pack-nl",
            "language-pack-gnome-nl",
            "hunspell-nl",
            "wdutch"
        ],
        "LibreOffice's only with libreoffice-common, the fonts rule names no prefix"
    );
    assert_eq!(
        languages::wanted(&rules, "zh-hans", &installed),
        [
            "language-pack-zh-hans",
            "language-pack-gnome-zh-hans",
            "hunspell-zh-hans",
            "fonts-noto-cjk"
        ]
    );
    let root = language_root();
    let b = root.buses();
    assert_eq!(languages::pack_code(&b, "zh_TW.UTF-8"), "zh-hant");
    assert_eq!(languages::pack_code(&b, "nl_BE.UTF-8"), "nl");
    let packs: std::collections::HashSet<String> = languages::installed_packs(&b);
    assert_eq!(
        packs,
        ["de".to_owned(), "en".to_owned()].into_iter().collect()
    );
    let all = languages::list(&b, &Mutex::new(None), Lang::En);
    let nl = all["languages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["code"] == "nl")
        .unwrap();
    assert_eq!(nl["installed"], false);
    assert_eq!(nl["locale"], "nl_NL.UTF-8");
    assert_eq!(
        nl["missing"],
        json!([
            "language-pack-nl",
            "language-pack-gnome-nl",
            "hunspell-nl",
            "wdutch"
        ])
    );
    let de = all["languages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["code"] == "de")
        .unwrap();
    assert_eq!(de["installed"], true);
    assert_eq!(de["missing"], json!(["hunspell-de", "wngerman"]));
    assert_eq!(de["complete"], true, "its language packs are there");
    assert_eq!(nl["complete"], false);
    let en = all["languages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["code"] == "en")
        .unwrap();
    assert_eq!(en["installed"], true, "English is the source language");
    assert_eq!(en["complete"], true, "and needs no language pack");
    assert_eq!(all["canInstall"], true);
    assert_eq!(all["installing"], Value::Null);
}

// --------------------------------------------------------------------------------------- the fakes (session bus)

const MACHINE_XML: &str = r#"<node>
  <interface name="org.freedesktop.hostname1">
    <method name="SetPrettyHostname"><arg type="s" direction="in"/><arg type="b" direction="in"/></method>
    <method name="SetStaticHostname"><arg type="s" direction="in"/><arg type="b" direction="in"/></method>
    <property name="Hostname" type="s" access="read"/>
    <property name="StaticHostname" type="s" access="read"/>
    <property name="PrettyHostname" type="s" access="read"/>
    <property name="Chassis" type="s" access="read"/>
    <property name="HardwareVendor" type="s" access="read"/>
    <property name="HardwareModel" type="s" access="read"/>
    <property name="FirmwareVersion" type="s" access="read"/>
  </interface>
  <interface name="org.freedesktop.timedate1">
    <method name="SetTime"><arg type="x" direction="in"/><arg type="b" direction="in"/><arg type="b" direction="in"/></method>
    <method name="SetTimezone"><arg type="s" direction="in"/><arg type="b" direction="in"/></method>
    <method name="SetNTP"><arg type="b" direction="in"/><arg type="b" direction="in"/></method>
    <property name="Timezone" type="s" access="read"/>
    <property name="NTP" type="b" access="read"/>
    <property name="CanNTP" type="b" access="read"/>
    <property name="NTPSynchronized" type="b" access="read"/>
    <property name="LocalRTC" type="b" access="read"/>
  </interface>
  <interface name="org.freedesktop.locale1">
    <method name="SetLocale"><arg type="as" direction="in"/><arg type="b" direction="in"/></method>
    <property name="Locale" type="as" access="read"/>
  </interface>
  <interface name="org.gnome.Shell">
    <property name="ShellVersion" type="s" access="read"/>
  </interface>
  <interface name="org.freedesktop.systemd1.Manager">
    <property name="Virtualization" type="s" access="read"/>
  </interface>
  <interface name="org.freedesktop.PackageKit">
    <method name="CreateTransaction"><arg type="o" direction="out"/></method>
  </interface>
  <interface name="org.freedesktop.PackageKit.Transaction">
    <method name="SetHints"><arg type="as" direction="in"/></method>
    <method name="Resolve"><arg type="t" direction="in"/><arg type="as" direction="in"/></method>
    <method name="InstallPackages"><arg type="t" direction="in"/><arg type="as" direction="in"/></method>
    <signal name="Package"><arg type="u"/><arg type="s"/><arg type="s"/></signal>
    <signal name="ErrorCode"><arg type="u"/><arg type="s"/></signal>
    <signal name="Finished"><arg type="u"/><arg type="u"/></signal>
  </interface>
</node>"#;

const TX: &str = "org.freedesktop.PackageKit.Transaction";

/// hostname1, timedate1 (NTP on, synchronised), locale1, GNOME Shell, systemd and PackageKit in one fake; SetNTP
/// changes the NTP property, CreateTransaction hands out /tx/1, /tx/2, ...
fn machine_fake() -> Option<(FakeBus, Arc<AtomicBool>)> {
    let ntp = Arc::new(AtomicBool::new(true));
    let next = Arc::new(std::sync::atomic::AtomicU32::new(1));
    let objects = vec![
        (HOSTNAME.1.to_owned(), HOSTNAME.2),
        (TIMEDATE.1.to_owned(), TIMEDATE.2),
        (LOCALE.1.to_owned(), LOCALE.2),
        (SHELL.1.to_owned(), SHELL.2),
        (SYSTEMD.1.to_owned(), SYSTEMD.2),
        (PACKAGEKIT.1.to_owned(), PACKAGEKIT.2),
        ("/tx/1".to_owned(), TX),
        ("/tx/2".to_owned(), TX),
    ];
    let ntp2 = ntp.clone();
    let methods: Methods = Arc::new(move |_, _, method, params| match method {
        "SetNTP" => {
            ntp2.store(
                params.child_value(0).get::<bool>().unwrap_or(false),
                Ordering::Release,
            );
            Ok(None)
        }
        "CreateTransaction" => {
            let n = next.fetch_add(1, Ordering::AcqRel);
            Ok(Some(
                (glib::variant::ObjectPath::try_from(format!("/tx/{n}")).unwrap(),).to_variant(),
            ))
        }
        _ => Ok(None),
    });
    let ntp3 = ntp.clone();
    let props: Props = Arc::new(move |_, iface, prop| {
        Some(match (iface, prop) {
            ("org.freedesktop.hostname1", "Hostname") => "werkplek".to_variant(),
            ("org.freedesktop.hostname1", "StaticHostname") => "werkplek".to_variant(),
            ("org.freedesktop.hostname1", "PrettyHostname") => "Werkplek".to_variant(),
            ("org.freedesktop.hostname1", "Chassis") => "laptop".to_variant(),
            ("org.freedesktop.hostname1", "HardwareVendor") => "LENOVO".to_variant(),
            ("org.freedesktop.hostname1", "HardwareModel") => "ThinkPad T14".to_variant(),
            ("org.freedesktop.hostname1", "FirmwareVersion") => "R1MET45W (1.15)".to_variant(),
            ("org.freedesktop.timedate1", "Timezone") => "Europe/Amsterdam".to_variant(),
            ("org.freedesktop.timedate1", "NTP") => ntp3.load(Ordering::Acquire).to_variant(),
            ("org.freedesktop.timedate1", "CanNTP") => true.to_variant(),
            ("org.freedesktop.timedate1", "NTPSynchronized") => true.to_variant(),
            ("org.freedesktop.timedate1", "LocalRTC") => false.to_variant(),
            ("org.freedesktop.locale1", "Locale") => vec![
                "LANG=en_US.UTF-8".to_owned(),
                "LC_TIME=nl_NL.UTF-8".to_owned(),
            ]
            .to_variant(),
            ("org.gnome.Shell", "ShellVersion") => "50.1".to_variant(),
            ("org.freedesktop.systemd1.Manager", "Virtualization") => "kvm".to_variant(),
            _ => return None,
        })
    });
    FakeBus::start(MACHINE_XML, objects, methods, props).map(|f| (f, ntp))
}

fn machine_on(fake: &FakeBus, dry: bool, root: Option<&Root>) -> (LangContext, Arc<Machine>) {
    let (lang, ctx) = context(dry);
    let peer = Peer {
        bus: common::session().unwrap(),
        name: fake.name.clone(),
    };
    let buses = Buses {
        hostname: Some(peer.clone()),
        timedate: Some(peer.clone()),
        locale: Some(peer.clone()),
        packagekit: Some(peer.clone()),
        systemd: Some(peer.clone()),
        shell: Some(peer.clone()),
        root: root.map(|r| r.0.clone()),
        ..Buses::default()
    };
    (lang, Machine::with(ctx, buses).unwrap())
}

fn call(m: &Machine, method: &str, args: Value) -> Result<Value, String> {
    m.call(method, &args)
        .expect("a machine method")
        .map_err(|e| e.key.unwrap_or(e.code))
}

#[test]
fn machine_writes_ask_polkit_and_carry_gnomes_arguments() {
    let Some((fake, _ntp)) = machine_fake() else {
        eprintln!("skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)");
        return;
    };
    let root = language_root();
    let (_lang, m) = machine_on(&fake, false, Some(&root));
    // the About details: hostname1, GNOME Shell's version, systemd's virtualization
    let info = call(&m, "machine.info", json!({})).unwrap();
    assert_eq!(info["prettyHostname"], "Werkplek");
    assert_eq!(info["model"], "LENOVO ThinkPad T14");
    assert_eq!(info["firmwareVersion"], "R1MET45W (1.15)");
    assert_eq!(info["gnomeVersion"], "50.1", "amendment 37: ShellVersion");
    assert_eq!(info["virtualization"], "KVM");
    assert_eq!(info["osBits"], 64);
    assert_eq!(info["canRename"], true);
    call(&m, "machine.setName", json!({"name": "Robins Laptop"})).unwrap();
    assert_eq!(
        call(&m, "machine.setName", json!({"name": "!!!"})).unwrap_err(),
        "machine.badName"
    );
    let t = call(&m, "machine.time.get", json!({})).unwrap();
    assert_eq!(
        (
            t["ntp"].as_bool(),
            t["synced"].as_bool(),
            t["timezone"].as_str()
        ),
        (Some(true), Some(true), Some("Europe/Amsterdam"))
    );
    assert_eq!(
        t["server"],
        Value::Null,
        "the fixture root has no chrony files"
    );
    // the manual time only while NTP is off, as GNOME
    assert_eq!(
        call(
            &m,
            "machine.time.setTime",
            json!({"usec": 1_794_839_700_000_000i64})
        )
        .unwrap_err(),
        "machine.ntpOn"
    );
    call(&m, "machine.time.sync", json!({})).unwrap();
    call(&m, "machine.time.setNtp", json!({"on": false})).unwrap();
    assert_eq!(
        call(&m, "machine.time.sync", json!({})).unwrap_err(),
        "machine.ntpOff",
        "Nu synchroniseren would switch NTP on"
    );
    call(
        &m,
        "machine.time.setTime",
        json!({"usec": 1_794_839_700_000_000i64}),
    )
    .unwrap();
    assert_eq!(
        call(&m, "machine.time.setTime", json!({"usec": 5})).unwrap_err(),
        "machine.badTime"
    );
    call(
        &m,
        "machine.time.setTimezone",
        json!({"zone": "Europe/Amsterdam"}),
    )
    .unwrap();
    assert_eq!(
        call(
            &m,
            "machine.time.setTimezone",
            json!({"zone": "Mars/Olympus"})
        )
        .unwrap_err(),
        "machine.badZone"
    );
    let login = call(&m, "machine.login.get", json!({})).unwrap();
    assert_eq!(
        (login["language"].as_str(), login["formats"].as_str()),
        (Some("en_US.UTF-8"), Some("nl_NL.UTF-8"))
    );
    call(
        &m,
        "machine.login.setLocale",
        json!({"locale": "nl_NL.UTF-8"}),
    )
    .unwrap();
    call(&m, "machine.login.setLocale", json!({"formats": ""})).unwrap();
    assert_eq!(
        call(&m, "machine.login.setLocale", json!({})).unwrap_err(),
        "machine.badLocale"
    );
    assert_eq!(
        call(&m, "machine.login.setLocale", json!({"locale": "nl; rm"})).unwrap_err(),
        "machine.badLocale"
    );
    let writes = fake.writes(&[]);
    assert_eq!(
        writes,
        vec![
            ("SetPrettyHostname".to_owned(), "('Robins Laptop', true)".to_owned(), true),
            ("SetStaticHostname".into(), "('robins-laptop', true)".into(), true),
            // amendment 30 and 68: Nu synchroniseren is SetNTP false, then true, and nothing else
            ("SetNTP".into(), "(false, true)".into(), true),
            ("SetNTP".into(), "(true, true)".into(), true),
            ("SetNTP".into(), "(false, true)".into(), true),
            ("SetTime".into(), "(int64 1794839700000000, false, true)".into(), true),
            ("SetTimezone".into(), "('Europe/Amsterdam', true)".into(), true),
            (
                "SetLocale".into(),
                "(['LANG=nl_NL.UTF-8', 'LC_TIME=nl_NL.UTF-8', 'LC_NUMERIC=nl_NL.UTF-8', 'LC_MONETARY=nl_NL.UTF-8', \
'LC_MEASUREMENT=nl_NL.UTF-8', 'LC_PAPER=nl_NL.UTF-8'], true)"
                    .into(),
                true
            ),
            ("SetLocale".into(), "(['LANG=en_US.UTF-8'], true)".into(), true),
        ]
    );
}

#[test]
fn a_dry_run_checks_the_arguments_and_calls_nothing() {
    let Some((fake, _ntp)) = machine_fake() else {
        eprintln!("skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)");
        return;
    };
    let root = language_root();
    let (_lang, m) = machine_on(&fake, true, Some(&root));
    for (method, args) in [
        ("machine.setName", json!({"name": "Robins Laptop"})),
        ("machine.time.setNtp", json!({"on": false})),
        (
            "machine.time.setTime",
            json!({"usec": 1_794_839_700_000_000i64}),
        ),
        (
            "machine.time.setTimezone",
            json!({"zone": "Europe/Amsterdam"}),
        ),
        ("machine.time.sync", json!({})),
        (
            "machine.login.setLocale",
            json!({"locale": "nl_NL.UTF-8", "formats": "en_GB.UTF-8"}),
        ),
        ("machine.languages.install", json!({"code": "nl"})),
    ] {
        let r = call(&m, method, args).unwrap();
        assert_eq!(r["dryRun"], true, "{method}");
    }
    // the checks still run
    assert_eq!(
        call(&m, "machine.setName", json!({"name": ""})).unwrap_err(),
        "machine.badName"
    );
    assert_eq!(
        call(&m, "machine.time.setTimezone", json!({"zone": "../x"})).unwrap_err(),
        "machine.badZone"
    );
    assert_eq!(
        call(&m, "machine.languages.install", json!({"code": "xx"})).unwrap_err(),
        "machine.badLanguage"
    );
    assert!(fake.writes(&[]).is_empty(), "{:?}", fake.writes(&[]));
}

#[test]
fn a_language_is_resolved_then_installed_through_packagekit() {
    let Some((fake, _ntp)) = machine_fake() else {
        eprintln!("skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)");
        return;
    };
    let root = language_root();
    let (_lang, m) = machine_on(&fake, false, Some(&root));
    let started = call(&m, "machine.languages.install", json!({"code": "nl"})).unwrap();
    assert_eq!(started["started"], true);
    assert_eq!(
        call(&m, "machine.languages.install", json!({"code": "de"})).unwrap_err(),
        "machine.installBusy",
        "one install at a time"
    );
    assert_eq!(
        call(&m, "machine.languages", json!({})).unwrap()["installing"],
        "nl"
    );
    // PackageKit's answers: the packages it has, then Finished
    assert!(fake.wait_for("Resolve"), "Resolve: {:?}", fake.calls());
    let emit = |path: &str, member: &str, args: glib::Variant| {
        fake.conn
            .emit_signal(None, path, TX, member, Some(&args))
            .unwrap();
        let _ = fake.conn.flush_sync(gio::Cancellable::NONE);
    };
    emit(
        "/tx/1",
        "Package",
        (2u32, "language-pack-nl;1:26.04;all;ubuntu", "").to_variant(),
    );
    emit(
        "/tx/1",
        "Package",
        (2u32, "wdutch;1:2.10;all;ubuntu", "").to_variant(),
    );
    // a multiarch system's foreign build (dpkg here is amd64): never installed with the language
    emit(
        "/tx/1",
        "Package",
        (
            2u32,
            "libenchant-2-voikko;2.8.2+dfsg1-3build1;i386;ubuntu",
            "",
        )
            .to_variant(),
    );
    emit("/tx/1", "Finished", (1u32, 10u32).to_variant());
    assert!(
        fake.wait_for("InstallPackages"),
        "InstallPackages: {:?}",
        fake.calls()
    );
    emit("/tx/2", "Finished", (1u32, 900u32).to_variant());
    let mut done = false;
    for _ in 0..300 {
        if call(&m, "machine.languages", json!({})).unwrap()["installing"].is_null() {
            done = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(done, "the install ends with Finished");
    let calls: Vec<(String, String, String, bool)> = fake
        .calls()
        .into_iter()
        .map(|c| (c.path, c.method, c.args, c.interactive))
        .collect();
    assert_eq!(
        calls,
        vec![
            ("/org/freedesktop/PackageKit".to_owned(), "CreateTransaction".to_owned(), "()".to_owned(), false),
            ("/tx/1".into(), "SetHints".into(), "(['interactive=false', 'background=false'],)".into(), false),
            (
                "/tx/1".into(),
                "Resolve".into(),
                "(uint64 327688, ['language-pack-nl', 'language-pack-gnome-nl', 'hunspell-nl', 'wdutch'])".into(),
                false
            ),
            ("/org/freedesktop/PackageKit".into(), "CreateTransaction".into(), "()".into(), false),
            ("/tx/2".into(), "SetHints".into(), "(['interactive=true', 'background=false'],)".into(), false),
            (
                "/tx/2".into(),
                "InstallPackages".into(),
                "(uint64 2, ['language-pack-nl;1:26.04;all;ubuntu', 'wdutch;1:2.10;all;ubuntu'])".into(),
                true
            ),
        ],
        "only what PackageKit knows for this architecture is installed (NOT_INSTALLED | NEWEST | ARCH, as \
         gnome-software resolves), with the password dialog allowed"
    );
}

#[test]
fn packagekit_ids_and_errors_read_as_gnome_software_reads_them() {
    assert_eq!(languages::FILTER_NOT_INSTALLED_NEWEST_ARCH, 327_688);
    assert_eq!(languages::native_arch(DPKG), "amd64");
    assert_eq!(
        languages::native_arch("Package: dpkg\nArchitecture: arm64\n"),
        "arm64"
    );
    assert!(!languages::native_arch("").is_empty(), "the build's own");
    assert!(languages::native_package(
        "wdutch;1:2.10;all;ubuntu",
        "amd64"
    ));
    assert!(languages::native_package(
        "hunspell-nl;1;amd64;ubuntu",
        "amd64"
    ));
    assert!(!languages::native_package(
        "hunspell-nl;1;i386;ubuntu",
        "amd64"
    ));
    let lang = crate::i18n::Lang::En;
    let message = |f: languages::Failure| match f {
        languages::Failure::Api(e) => e.key.unwrap_or_default(),
        languages::Failure::Key(k) => k.to_owned(),
        languages::Failure::PackageKit(d) => format!("pk:{d}"),
    };
    assert_eq!(
        message(languages::error_failure(
            48,
            "Failed to obtain authentication.".into(),
            lang
        )),
        "core.notAuthorized"
    );
    assert_eq!(
        message(languages::error_failure(17, "cancelled".into(), lang)),
        "machine.installStopped"
    );
    assert_eq!(
        message(languages::error_failure(13, "dependency".into(), lang)),
        "pk:dependency"
    );
}

#[test]
fn a_dismissed_password_dialog_is_no_failed_install() {
    let Some((fake, _ntp)) = machine_fake() else {
        eprintln!("skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)");
        return;
    };
    let root = language_root();
    let (lang, m) = machine_on(&fake, false, Some(&root));
    let seen: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    lang.ctx.events.on(
        "machine.changed",
        Arc::new(move |d: &Value| seen2.lock().unwrap().push(d.clone())),
    );
    assert_eq!(
        call(&m, "machine.languages.install", json!({"code": "nl"})).unwrap()["started"],
        true
    );
    let emit = |path: &str, member: &str, args: glib::Variant| {
        fake.conn
            .emit_signal(None, path, TX, member, Some(&args))
            .unwrap();
        let _ = fake.conn.flush_sync(gio::Cancellable::NONE);
    };
    assert!(fake.wait_for("Resolve"), "Resolve: {:?}", fake.calls());
    emit(
        "/tx/1",
        "Package",
        (2u32, "language-pack-nl;1:26.04;all;ubuntu", "").to_variant(),
    );
    emit("/tx/1", "Finished", (1u32, 10u32).to_variant());
    assert!(fake.wait_for("InstallPackages"), "{:?}", fake.calls());
    // PackageKit answers InstallPackages before polkit decides; the dismissed dialog comes as ErrorCode, then Finished
    emit(
        "/tx/2",
        "ErrorCode",
        (48u32, "Failed to obtain authentication.").to_variant(),
    );
    emit("/tx/2", "Finished", (2u32, 0u32).to_variant());
    let mut done = None;
    for _ in 0..300 {
        done = seen
            .lock()
            .unwrap()
            .iter()
            .find(|d| d["done"] == true)
            .cloned();
        if done.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let done = done.expect("the install's end");
    assert_eq!(done["ok"], false);
    assert_eq!(
        done["error"],
        crate::i18n::tr(crate::i18n::Lang::En, "core.notAuthorized"),
        "the polkit refusal in the UI's language, not PackageKit's English as an install failure"
    );
}

#[test]
fn every_machine_method_is_known_and_unknown_ones_are_not_answered() {
    let (_lang, ctx) = context(false);
    let root = Root::new();
    let m = Machine::with(ctx, root.buses()).unwrap();
    assert!(m.call("machine.nonsense", &json!({})).is_none());
    // without the daemons (the sandbox has no system bus): unavailable, never a hang
    let t = call(&m, "machine.time.get", json!({})).unwrap();
    assert_eq!(t["available"], false);
    assert_eq!(
        call(&m, "machine.time.setNtp", json!({"on": true})).unwrap_err(),
        "machine.timeUnavailable"
    );
    assert_eq!(
        call(&m, "machine.login.get", json!({})).unwrap()["available"],
        false
    );
    assert_eq!(
        call(&m, "machine.info", json!({})).unwrap()["canRename"],
        false
    );
    let mut names: Vec<&str> = METHODS.iter().map(|(n, _)| *n).collect();
    let sorted = {
        let mut s = names.clone();
        s.sort();
        s
    };
    assert_eq!(names, sorted, "METHODS is sorted");
    names.dedup();
    assert_eq!(names.len(), METHODS.len());
    for (name, dangerous) in METHODS {
        let reads = [
            "machine.info",
            "machine.languages",
            "machine.locales",
            "machine.login.get",
            "machine.time.get",
            "machine.time.zones",
        ];
        assert_eq!(*dangerous, !reads.contains(name), "{name}");
    }
}

// ------------------------------------------------------------------------- tests/dbus-fakes (system bus)

#[test]
#[ignore = "tests/dbus-fakes/run.sh --batch e: hostname1, timedate1, locale1 and PackageKit fakes on a private system bus"]
fn fakes_machine_on_the_system_bus() {
    let (_lang, ctx) = context(false);
    let root = language_root();
    // the daemons are the system bus's own (fakes/e/machine.py); only the data files come from the fixture root
    let m = Machine::with(ctx, root.buses()).unwrap();
    let info = call(&m, "machine.info", json!({})).unwrap();
    assert_eq!(
        (info["prettyHostname"].as_str(), info["canRename"].as_bool()),
        (Some("Testpc"), Some(true))
    );
    call(&m, "machine.setName", json!({"name": "Robins Laptop"})).unwrap();
    let t = call(&m, "machine.time.get", json!({})).unwrap();
    assert_eq!(
        (t["available"].as_bool(), t["ntp"].as_bool()),
        (Some(true), Some(true))
    );
    // amendment 68: Nu synchroniseren without timesync1 calls only timedate1 SetNTP
    call(&m, "machine.time.sync", json!({})).unwrap();
    call(
        &m,
        "machine.time.setTimezone",
        json!({"zone": "Europe/Amsterdam"}),
    )
    .unwrap();
    call(
        &m,
        "machine.login.setLocale",
        json!({"locale": "nl_NL.UTF-8", "formats": "nl_NL.UTF-8"}),
    )
    .unwrap();
    call(&m, "machine.languages.install", json!({"code": "nl"})).unwrap();
    let mut done = false;
    for _ in 0..500 {
        if call(&m, "machine.languages", json!({})).unwrap()["installing"].is_null() {
            done = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(done, "the fake's Finished ends the install");
    let reads = ["CreateTransaction", "SetHints", "Resolve"];
    let writes: Vec<(String, String, bool)> = fake_calls("WIN11_FAKE_CALLS_MACHINE")
        .into_iter()
        .filter(|c| !reads.contains(&c.0.as_str()))
        .collect();
    assert_eq!(
        writes,
        vec![
            ("SetPrettyHostname".to_owned(), "('Robins Laptop', true)".to_owned(), true),
            ("SetStaticHostname".into(), "('robins-laptop', true)".into(), true),
            ("SetNTP".into(), "(false, true)".into(), true),
            ("SetNTP".into(), "(true, true)".into(), true),
            ("SetTimezone".into(), "('Europe/Amsterdam', true)".into(), true),
            (
                "SetLocale".into(),
                "(['LANG=nl_NL.UTF-8', 'LC_TIME=nl_NL.UTF-8', 'LC_NUMERIC=nl_NL.UTF-8', 'LC_MONETARY=nl_NL.UTF-8', \
'LC_MEASUREMENT=nl_NL.UTF-8', 'LC_PAPER=nl_NL.UTF-8'], true)"
                    .into(),
                true
            ),
            (
                "InstallPackages".into(),
                "(uint64 2, ['language-pack-nl;1:26.04;all;fake', 'language-pack-gnome-nl;1:26.04;all;fake', \
'hunspell-nl;1:26.04;all;fake', 'wdutch;1:26.04;all;fake'])"
                    .into(),
                true
            ),
        ]
    );
    let all = fake_calls("WIN11_FAKE_CALLS_MACHINE");
    assert!(
        !all.iter().any(|c| c.0.contains("timesync")),
        "no timesync1 call: {all:?}"
    );
}

#[test]
fn info_lists_the_drives_of_its_disk_capacity() {
    // UDisks2's GetManagedObjects as JSON: two fixed drives, a hot-swap bay drive UDisks counts as removable and a
    // USB stick; GNOME's capacity sums the first two, and Info lists those two only
    let drive = |vendor: &str, model: &str, size: u64, removable: bool, ejectable: bool| {
        json!({"org.freedesktop.UDisks2.Drive": {"Vendor": vendor, "Model": model, "Size": size,
            "Removable": removable, "Ejectable": ejectable}})
    };
    let objects = json!({
        "/org/freedesktop/UDisks2/drives/A": drive("", "ST24000DM001-3Y7103", 24_000_277_250_048, false, false),
        "/org/freedesktop/UDisks2/drives/B": drive("Lexar", "SSD NM790 1TB", 1_024_209_543_168, false, false),
        "/org/freedesktop/UDisks2/drives/C": drive("", "HGST HTS545050A7E680", 500_107_862_016, true, false),
        "/org/freedesktop/UDisks2/drives/D": drive("Kingston", "DataTraveler", 31_000_000_000, true, true),
        "/org/freedesktop/UDisks2/block_devices/sda": {"org.freedesktop.UDisks2.Block": {"Size": 1}},
    });
    let drives = info::disk_drives(&objects);
    assert_eq!(
        drives,
        vec![
            json!({"model": "ST24000DM001-3Y7103", "size": 24_000_277_250_048u64}),
            json!({"model": "Lexar SSD NM790 1TB", "size": 1_024_209_543_168u64}),
        ]
    );
    let sum: u64 = drives.iter().filter_map(|d| d["size"].as_u64()).sum();
    assert_eq!(
        info::disk_capacity(&objects),
        Some(sum),
        "the list adds up to the total"
    );
    assert_eq!(info::disk_capacity(&Value::Null), None);
    assert!(info::disk_drives(&Value::Null).is_empty());
}
