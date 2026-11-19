//! Batch B's `printers.*` tests: the pure parsers on fixtures (IPP answers, PPDs, DevicesGet, DNS-SD records, the
//! typed address, the driver ranking, lpoptions), the argument checks and the dry run, libcups against an IPP
//! responder of its own on loopback (the scheduler's answers and the requests it got), and the admin writes
//! against a cups-pk-helper fake on the private session bus (devices/fake_bus.rs: the calls recorded with the
//! interactive flag).
use super::*;
use crate::services::devices::fake_bus::{Fake, Handler, Object, Props};
use crate::services::system::common::LangContext;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};

fn lang() -> Lang {
    Lang::En
}

fn attrs(v: Value) -> cups::Attrs {
    v.as_object().unwrap().clone()
}

// ------------------------------------------------------------------------------------------- pure parsers

#[test]
fn a_queue_reads_as_gnomes_printer_row() {
    let a = attrs(json!({
        "printer-name": ["Epson_ET_2850"], "printer-info": ["Epson ET-2850"], "printer-location": ["Studeerkamer"],
        "printer-make-and-model": ["Epson ET-2850 Series"], "printer-state": [5],
        "printer-state-reasons": ["media-empty-error", "marker-supply-low-warning", "paused", "none"],
        "printer-is-accepting-jobs": [true], "printer-type": [0x0080D04C], "device-uri": ["usb://EPSON/ET-2850"],
        "marker-names": ["Cyan", "Black", "Waste"], "marker-levels": [12, 90, -1],
        "marker-colors": ["#00FFFF", "#000000", "none"], "marker-types": ["ink-tank", "ink-tank", "waste-ink"],
        "printer-commands": ["none"], "queued-job-count": [2],
    }));
    let p = ipp::printer(&a, Some("Epson_ET_2850"));
    assert_eq!(p["title"], "Epson ET-2850");
    assert_eq!(p["state"], "stopped");
    assert_eq!(p["isDefault"], true);
    assert_eq!(p["canClean"], false);
    assert_eq!(p["jobs"], 2);
    assert_eq!(
        p["reasons"],
        json!([{"reason": "media-empty", "severity": "error"},
               {"reason": "marker-supply-low", "severity": "warning"},
               {"reason": "paused", "severity": "report"}])
    );
    let markers = p["markers"].as_array().unwrap();
    assert_eq!(
        markers[0],
        json!({"name": "Cyan", "level": 12, "colors": ["#00ffff"], "type": "ink-tank"})
    );
    assert_eq!(
        markers[2]["level"],
        Value::Null,
        "a level below 0 is unknown"
    );
    assert_eq!(markers[2]["colors"], json!([]));
    assert_eq!(p["commands"], json!([]));
    // a queue without a printer-info is titled by its name; Clean among the commands
    let b = attrs(
        json!({"printer-name": ["HP"], "printer-info": [""], "printer-state": [3],
        "printer-commands": ["AutoConfigure", "Clean"], "printer-type": [1]}),
    );
    let p = ipp::printer(&b, None);
    assert_eq!(
        (
            p["title"].clone(),
            p["state"].clone(),
            p["canClean"].clone(),
            p["isClass"].clone()
        ),
        (json!("HP"), json!("idle"), json!(true), json!(true))
    );
    // CUPS's temporary queues are not installed printers
    assert!(!ipp::listed(&attrs(
        json!({"printer-name": ["Tmp"], "printer-is-temporary": [true]})
    )));
    assert!(ipp::listed(&b));
}

#[test]
fn a_job_reads_as_gnomes_jobs_dialog() {
    let j = ipp::job(&attrs(json!({
        "job-id": [42], "job-name": ["Foto.jpg"], "job-state": [4],
        "job-state-reasons": ["cups-held-for-authentication"], "job-originating-user-name": ["robin"],
        "job-printer-uri": ["ipp://localhost/printers/HP%20Laser"], "job-priority": [50], "job-k-octets": [1800],
        "time-at-creation": [1763280000], "job-hold-until": ["auth-info-required"],
    })));
    assert_eq!(j["id"], 42);
    assert_eq!(j["state"], "held");
    assert_eq!(j["held"], true);
    assert_eq!(j["authRequired"], true);
    assert_eq!(
        j["printer"], "HP Laser",
        "the queue name decoded from the URI"
    );
    assert_eq!(j["done"], false);
    let done = ipp::job(&attrs(
        json!({"job-id": [7], "job-state": [9], "job-state-reasons": ["none"]}),
    ));
    assert_eq!(
        (
            done["state"].clone(),
            done["done"].clone(),
            done["reasons"].clone()
        ),
        (json!("completed"), json!(true), json!([]))
    );
}

#[test]
fn driverless_is_ipp_2_with_a_raster_or_pdf_format() {
    let everywhere = attrs(json!({"ipp-versions-supported": ["1.1", "2.0"],
        "document-format-supported": ["application/octet-stream", "image/pwg-raster"]}));
    assert!(ipp::driverless(&everywhere));
    let old = attrs(json!({"ipp-versions-supported": ["1.0", "1.1"],
        "document-format-supported": ["application/pdf"]}));
    assert!(!ipp::driverless(&old), "IPP 1.1 only");
    let pcl = attrs(json!({"ipp-versions-supported": ["2.0"],
        "document-format-supported": ["application/vnd.hp-PCL"]}));
    assert!(!ipp::driverless(&pcl));
    assert_eq!(
        ipp::device_id("MFG:HP;MDL:LaserJet M404;CMD:PCL;"),
        ("HP".into(), "LaserJet M404".into())
    );
    assert_eq!(
        ipp::device_id("MANUFACTURER:EPSON;MODEL:ET-2850;"),
        ("EPSON".into(), "ET-2850".into())
    );
}

const PPD: &str = r#"*PPD-Adobe: "4.3"
*LanguageEncoding: ISOLatin1
*cupsLanguages: "nl"
*OpenUI *PageRegion: PickOne
*PageRegion A4/A4: "<</PageSize[595 842]>>setpagedevice"
*CloseUI: *PageRegion
*OpenGroup: General/General
*OpenUI *PageSize/Media Size: PickOne
*OrderDependency: 10 AnySetup *PageSize
*DefaultPageSize: Letter
*PageSize A4/A4: "<</PageSize[595 842]>>setpagedevice"
*PageSize Letter/US Letter: "<</PageSize
[612 792]>>setpagedevice"
*End
*CloseUI: *PageSize
*OpenUI *Duplex/2-Sided Printing: PickOne
*DefaultDuplex: None
*Duplex None/Off: ""
*Duplex DuplexNoTumble/Long Edge: ""
*CloseUI: *Duplex
*CloseGroup: General
*OpenUI *Collate/Collate Copies: Boolean
*DefaultCollate: True
*Collate True/On: ""
*Collate False/Off: ""
*CloseUI: *Collate
*OpenGroup: Quality/Print Quality
*JCLOpenUI *JCLQuality/Qualit<E9>: PickOne
*DefaultJCLQuality: Normal
*JCLQuality Normal/Normal: "@PJL SET QUALITY=NORMAL"
*JCLQuality High/Hoog: "@PJL SET QUALITY=HIGH"
*JCLCloseUI: *JCLQuality
*CloseGroup: Quality
*nl.Translation General/Algemeen: ""
*nl.Translation PageSize/Papierformaat: ""
*nl.PageSize Letter/US-Letter: ""
*nl.Translation Duplex/Dubbelzijdig: ""
*nl_NL.Duplex None/Uit: ""
*de.Translation PageSize/Papiergr<F6>sse: ""
"#;

#[test]
fn a_ppd_reads_as_gnomes_printing_options() {
    let groups = ppd::parse(PPD, "nl");
    let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(
        names,
        ["General", "Quality"],
        "PageRegion is hidden; Collate joins CUPS's General group"
    );
    let general = &groups[0];
    assert_eq!(general.text, "Algemeen");
    let keywords: Vec<&str> = general.options.iter().map(|o| o.keyword.as_str()).collect();
    assert_eq!(keywords, ["PageSize", "Duplex", "Collate"]);
    let size = &general.options[0];
    assert_eq!(
        (size.text.as_str(), size.default.as_str(), size.ui.as_str()),
        ("Papierformaat", "Letter", "pickone")
    );
    assert_eq!(
        size.choices,
        vec![
            ppd::Choice {
                value: "A4".into(),
                text: "A4".into()
            },
            ppd::Choice {
                value: "Letter".into(),
                text: "US-Letter".into()
            },
        ],
        "a value over two lines does not end the option"
    );
    assert_eq!(
        general.options[1].choices[0].text, "Uit",
        "nl_NL is Dutch too"
    );
    assert_eq!(general.options[2].ui, "boolean");
    let quality = &groups[1].options[0];
    assert_eq!(quality.keyword, "JCLQuality");
    assert_eq!(quality.text, "Qualité", "<E9> in ISO 8859-1");
    // English: the PPD's own texts, no other language's
    let en = ppd::parse(PPD, "en");
    assert_eq!(en[0].text, "General");
    assert_eq!(en[0].options[0].text, "Media Size");
    // the queue's PageSize-default wins over the PPD's own default
    let json = ppd::to_json(&groups, &|k: &str| {
        (k == "PageSize").then(|| "A4".to_owned())
    });
    assert_eq!(json[0]["options"][0]["value"], "A4");
    assert_eq!(json[0]["options"][1]["value"], "None");
    let json = ppd::to_json(&groups, &|_: &str| Some("Nonsense".to_owned()));
    assert_eq!(
        json[0]["options"][0]["value"], "Letter",
        "a default that names no choice is ignored"
    );
    assert!(ppd::has_option(&groups, "Duplex") && !ppd::has_option(&groups, "InputSlot"));
    assert_eq!(ppd::decode_hex("A<C3A9>B <zz>"), "AéB <zz>");
}

#[test]
fn odd_hex_in_a_ppd_stays_text() {
    // review B finding 5: an odd number of digits once the spaces are gone panicked the request thread
    assert_eq!(ppd::decode_hex("Tray <1 >"), "Tray <1 >");
    assert_eq!(ppd::decode_hex("Kleur <E9 2>"), "Kleur <E9 2>");
    assert_eq!(ppd::decode_hex("A<>B"), "A<>B");
    assert_eq!(ppd::decode_hex("<E"), "<E");
    assert_eq!(ppd::decode_hex("Qualit<E9>"), "Qualité");
    assert_eq!(ppd::decode_hex("<C3 A9>t<C3A9>"), "été");
    // a whole PPD with them parses: the choices keep their literal text
    const SLOTS: &str = "*PPD-Adobe: \"4.3\"\n*OpenUI *InputSlot/Paper Source: PickOne\n*DefaultInputSlot: Tray1\n*InputSlot Tray1/Tray <1 >: \"\"\n*InputSlot Tray2/Lade <E9 2>: \"\"\n*CloseUI: *InputSlot\n*nl.InputSlot Tray2/Lade <E9 2>: \"\"\n";
    let slots = ppd::parse(SLOTS, "nl")
        .into_iter()
        .flat_map(|g| g.options)
        .find(|o| o.keyword == "InputSlot")
        .unwrap();
    let texts: Vec<&str> = slots.choices.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(texts, ["Tray <1 >", "Lade <E9 2>"]);
}

#[test]
fn devices_get_reads_cups_pk_helpers_numbered_keys() {
    let map = json!({
        "device-class:0": "direct", "device-uri:0": "usb://HP/DeskJet%202700?serial=CN1", "device-info:0": "HP DeskJet 2700",
        "device-make-and-model:0": "HP DeskJet 2700 series", "device-id:0": "MFG:HP;MDL:DeskJet 2700 series;",
        "device-location:0": "",
        "device-class:1": "network", "device-uri:1": "socket", "device-info:1": "AppSocket/HP JetDirect",
        "device-class:2": "network", "device-uri:2": "ipp://printer.local:631/ipp/print", "device-info:2": "Office",
        "device-make-and-model:2": "Unknown", "device-id:2": "MFG:Brother;MDL:HL-L2350DW;",
    });
    let devices = discover::devices_get(map.as_object().unwrap());
    assert_eq!(
        devices.len(),
        2,
        "a backend's bare scheme is no device: {devices:?}"
    );
    assert_eq!(devices[0]["uri"], "usb://HP/DeskJet%202700?serial=CN1");
    assert_eq!(
        (
            devices[0]["network"].clone(),
            devices[0]["driverless"].clone()
        ),
        (json!(false), json!(false))
    );
    assert_eq!(
        devices[1]["makeModel"], "Brother HL-L2350DW",
        "Unknown falls back to the IEEE 1284 id"
    );
    assert_eq!(devices[1]["driverless"], true);
}

#[test]
fn a_dns_sd_printer_is_driverless_by_its_pdl() {
    let txt = discover::txt_record(&[
        b"rp=ipp/print".to_vec(),
        b"ty=HP LaserJet M404".to_vec(),
        b"pdl=application/octet-stream,image/urf,image/pwg-raster".to_vec(),
        b"note=Kantoor".to_vec(),
        b"usb_MFG=HP".to_vec(),
        b"usb_MDL=LaserJet M404".to_vec(),
    ]);
    let d = discover::dnssd_device("HP LaserJet", "_ipps._tcp", "hp.local.", 631, &txt);
    assert_eq!(d["uri"], "ipps://hp.local:631/ipp/print");
    assert_eq!(d["driverless"], true);
    assert_eq!(d["deviceId"], "MFG:HP;MDL:LaserJet M404;");
    assert_eq!(d["location"], "Kantoor");
    let pcl = discover::txt_record(&[b"rp=queue".to_vec(), b"pdl=application/vnd.hp-PCL".to_vec()]);
    assert_eq!(
        discover::dnssd_device("Old", "_ipp._tcp", "h", 631, &pcl)["driverless"],
        false
    );
    let shared = discover::txt_record(&[
        b"rp=printers/Laser".to_vec(),
        b"printer-type=0x809046".to_vec(),
    ]);
    let d = discover::dnssd_device("Laser @ server", "_ipp._tcp", "server.local", 631, &shared);
    assert_eq!(
        (d["remoteQueue"].clone(), d["driverless"].clone()),
        (json!(true), json!(true))
    );
}

#[test]
fn a_typed_address_is_a_uri_or_a_host() {
    use discover::Target;
    assert_eq!(
        discover::target("192.168.1.20"),
        Some(Target::Host("192.168.1.20".into(), None))
    );
    assert_eq!(
        discover::target(" printer.local:8631 "),
        Some(Target::Host("printer.local".into(), Some(8631)))
    );
    assert_eq!(
        discover::target("[fe80::1]:631"),
        Some(Target::Host("fe80::1".into(), Some(631)))
    );
    assert_eq!(
        discover::target("fe80::1"),
        Some(Target::Host("fe80::1".into(), None))
    );
    assert_eq!(
        discover::target("IPP://h/ipp/print"),
        Some(Target::Uri("ipp://h/ipp/print".into()))
    );
    assert_eq!(
        discover::target("smb://server/share"),
        Some(Target::Uri("smb://server/share".into()))
    );
    for bad in [
        "",
        "a b",
        "file:///etc/passwd",
        "ftp://x",
        "h:port",
        "x;y",
        "usb://HP",
        "smb://",
    ] {
        assert_eq!(discover::target(bad), None, "{bad:?}");
    }
    assert_eq!(
        discover::uri_host("ipp://u@h:632/x"),
        Some(("h".into(), Some(632)))
    );
    assert_eq!(
        discover::uri_host("socket://[::1]:9100"),
        Some(("::1".into(), Some(9100)))
    );
    assert_eq!(
        cups::Server::parse("127.0.0.1:8631"),
        Some(("127.0.0.1".into(), 8631))
    );
    assert_eq!(
        cups::Server::parse("/run/cups/cups.sock"),
        Some(("/run/cups/cups.sock".into(), 0))
    );
    assert_eq!(
        cups::Server::parse("cups.example"),
        Some(("cups.example".into(), 631))
    );
}

#[test]
fn drivers_rank_by_the_device_id_then_the_model() {
    let d = |name: &str, make: &str, mm: &str, id: &str| json!({"name": name, "make": make, "makeModel": mm, "deviceId": id});
    let drivers = vec![
        d("everywhere", "Generic", "IPP Everywhere", ""),
        d(
            "drv:///sample.drv/generic.ppd",
            "Generic",
            "Generic PostScript Printer",
            "",
        ),
        d(
            "hp-m404.ppd",
            "HP",
            "HP LaserJet Pro M404-M405, hpcups 3.23",
            "MFG:HP;MDL:HP LaserJet Pro M404-M405;",
        ),
        d("hp-m402.ppd", "HP", "HP LaserJet Pro M402, hpcups 3.23", ""),
        d("hp-dj.ppd", "HP", "HP DeskJet 2700 Series, hpcups 3.23", ""),
        d(
            "epson.ppd",
            "EPSON",
            "Epson ET-2850 Series, ESC/P-R",
            "MFG:EPSON;MDL:ET-2850 Series;",
        ),
    ];
    let best = discover::best(&drivers, "MFG:HP;MDL:HP LaserJet Pro M404-M405;", "", false);
    assert_eq!(best[0]["name"], "hp-m404.ppd");
    assert_eq!(best[0]["score"], 100);
    let best = discover::best(&drivers, "", "Epson ET-2850 Series", true);
    assert_eq!(
        best[0]["name"], "everywhere",
        "a driverless device gets IPP Everywhere first"
    );
    assert_eq!(best[1]["name"], "epson.ppd");
    let best = discover::best(&drivers, "", "HP LaserJet Pro M402dn", false);
    assert_eq!(
        best.first().map(|b| b["name"].clone()),
        Some(json!("hp-m402.ppd")),
        "{best:?}"
    );
    assert!(discover::best(&drivers, "", "Brother HL-L2350DW", false).is_empty());
}

#[test]
fn lpoptions_follows_gnomes_printer_set_default() {
    assert_eq!(
        lpoptions_default("Dest A x=1\nDefault B/draft sides=one-sided\n").as_deref(),
        Some("B")
    );
    assert_eq!(lpoptions_default("Dest A\n"), None);
    // this computer's scheduler (review B finding 4): the user's default goes, no Default line comes
    assert_eq!(
        lpoptions_without_default("Dest A\n"),
        None,
        "no user default: nothing to write"
    );
    assert_eq!(
        lpoptions_without_default("Default Epson\nDest HP_LaserJet sides=one-sided\n").unwrap(),
        "Dest Epson\nDest HP_LaserJet sides=one-sided\n"
    );
    assert_eq!(
        lpoptions_without_default("Default B/draft media=a4\n").unwrap(),
        "Dest B/draft media=a4\n",
        "an instance keeps its options"
    );
    // another computer's scheduler: only the user's default changes
    assert_eq!(
        lpoptions_with_default(
            "Default B media=a4\nDest A sides=two-sided-long-edge\n",
            "A"
        ),
        "Dest B media=a4\nDefault A sides=two-sided-long-edge\n"
    );
    assert_eq!(
        lpoptions_with_default("Default B\n", "C"),
        "Dest B\nDefault C\n"
    );
    assert_eq!(lpoptions_with_default("", "C"), "Default C\n");
    for local in [
        "localhost",
        "LOCALHOST:631",
        "127.0.0.1",
        "::1",
        "/run/cups/cups.sock",
    ] {
        assert!(cups::is_local(local), "{local}");
    }
    for remote in ["print.example", "192.168.1.20:631", "[fe80::1]"] {
        assert!(!cups::is_local(remote), "{remote}");
    }
}

#[test]
fn clear_all_cancels_every_job_on_its_own() {
    // review B finding 6: one refusal keeps no other job in the queue
    let no = || {
        ApiError::tr(
            "denied",
            lang(),
            "core.notAuthorized",
            crate::i18n::NO_PARAMS,
        )
    };
    let mixed = clear_answer(vec![Err(no()), Ok(())]).unwrap();
    assert_eq!(mixed, json!({"ok": true, "canceled": 1, "failed": 1}));
    let all = clear_answer(vec![Err(no()), Err(no())]).unwrap_err();
    assert_eq!(all.key.as_deref(), Some("core.notAuthorized"));
    assert_eq!(
        clear_answer(vec![]).unwrap(),
        json!({"ok": true, "canceled": 0, "failed": 0})
    );
}

#[test]
fn arguments_are_checked_as_cups_takes_them() {
    for ok in ["HP_LaserJet", "Epson-ET.2850", "Drukker@kantoor"] {
        assert!(name_ok(ok), "{ok}");
    }
    for bad in [
        "",
        "a b",
        "a/b",
        "a#b",
        "a\tb",
        "a\"b",
        "a'b",
        "a?b",
        "a\\b",
        &"x".repeat(128),
    ] {
        assert!(!name_ok(bad), "{bad:?}");
    }
    assert!(
        uri_ok("ipp://h/ipp/print") && uri_ok("usb://HP/DJ?serial=1") && uri_ok("socket://h:9100")
    );
    for bad in ["", "nouri", "IPP://x", "ipp:", "a b://x", "ipp://a b"] {
        assert!(!uri_ok(bad), "{bad:?}");
    }
    assert!(ppd_name_ok("everywhere") && ppd_name_ok("drv:///sample.drv/generic.ppd"));
    assert!(!ppd_name_ok("../../etc/shadow") && !ppd_name_ok("a b"));
    let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    assert!(option_ok("PageSize", &v(&["A4"])) && option_ok("number-up", &v(&["2"])));
    assert!(
        !option_ok("PageSize-default", &v(&["A4"])),
        "the mechanism adds -default itself"
    );
    assert!(
        !option_ok("Page Size", &v(&["A4"]))
            && !option_ok("PageSize", &[])
            && !option_ok("x", &v(&[""]))
    );
    let check = |m: &str, a: Value| Printers::check(m, &a, lang());
    assert!(
        check(
            "printers.add",
            json!({"name": "HP", "uri": "ipp://h/ipp/print", "ppd": "everywhere"})
        )
        .is_ok()
    );
    assert_eq!(
        check("printers.add", json!({"name": "H P", "uri": "ipp://h"}))
            .unwrap_err()
            .key
            .as_deref(),
        Some("printers.badName")
    );
    assert!(
        check(
            "printers.add",
            json!({"name": "HP", "uri": "ipp://h", "ppd": "x", "ppdFile": "/etc/hostname"})
        )
        .is_err(),
        "a PPD name and a file at once"
    );
    assert_eq!(
        check(
            "printers.add",
            json!({"name": "HP", "uri": "ipp://h", "ppdFile": "/no/such.ppd"})
        )
        .unwrap_err()
        .key
        .as_deref(),
        Some("printers.badFile")
    );
    let dir = std::env::temp_dir().join(format!("win11-ppd-{}", crate::events::nonce()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ok.ppd"), "*PPD-Adobe: \"4.3\"\n").unwrap();
    std::fs::write(dir.join("ok.ppd.gz"), [0x1f, 0x8b, 8, 0]).unwrap();
    std::fs::write(dir.join("text.txt"), "not a PPD").unwrap();
    assert!(ppd_file_ok(&dir.join("ok.ppd")) && ppd_file_ok(&dir.join("ok.ppd.gz")));
    assert!(
        !ppd_file_ok(&dir.join("text.txt")) && !ppd_file_ok(&dir),
        "only a PPD the user can read"
    );
    let not_ppd =
        json!({"name": "HP", "uri": "ipp://h", "ppdFile": dir.join("text.txt").to_str().unwrap()});
    assert_eq!(
        check("printers.add", not_ppd).unwrap_err().key.as_deref(),
        Some("printers.badFile")
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert!(check("printers.job.priority", json!({"id": 4, "priority": 100})).is_ok());
    assert!(check("printers.job.priority", json!({"id": 4, "priority": 101})).is_err());
    assert!(check("printers.job.cancel", json!({"id": 0})).is_err());
    assert!(check("printers.jobs", json!({"which": "all"})).is_ok());
    assert!(check("printers.jobs", json!({"which": "mine"})).is_err());
    assert_eq!(
        check("printers.discover", json!({"address": "a b"}))
            .unwrap_err()
            .key
            .as_deref(),
        Some("printers.badAddress")
    );
    assert!(check("printers.setEnabled", json!({"name": "HP", "on": "yes"})).is_err());
}

// ------------------------------------------------------------------------- an IPP responder for libcups

/// A value of an IPP answer.
#[derive(Clone)]
enum V {
    I(i32),
    E(i32),
    B(bool),
    S(u8, String),
}

fn s(tag: u8, v: &str) -> V {
    V::S(tag, v.into())
}

fn encode(status: u16, id: u32, groups: &[(u8, Vec<(&str, Vec<V>)>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0x0200u16.to_be_bytes());
    out.extend_from_slice(&status.to_be_bytes());
    out.extend_from_slice(&id.to_be_bytes());
    let head = vec![
        ("attributes-charset", vec![s(0x47, "utf-8")]),
        ("attributes-natural-language", vec![s(0x48, "en")]),
    ];
    for (group, attrs) in std::iter::once(&(0x01u8, head)).chain(groups.iter()) {
        out.push(*group);
        for (name, values) in attrs {
            for (i, v) in values.iter().enumerate() {
                let (tag, raw): (u8, Vec<u8>) = match v {
                    V::I(n) => (0x21, n.to_be_bytes().to_vec()),
                    V::E(n) => (0x23, n.to_be_bytes().to_vec()),
                    V::B(b) => (0x22, vec![*b as u8]),
                    V::S(t, s) => (*t, s.as_bytes().to_vec()),
                };
                out.push(tag);
                let n = if i == 0 { name.as_bytes() } else { b"" };
                out.extend_from_slice(&(n.len() as u16).to_be_bytes());
                out.extend_from_slice(n);
                out.extend_from_slice(&(raw.len() as u16).to_be_bytes());
                out.extend_from_slice(&raw);
            }
        }
    }
    out.push(0x03);
    out
}

/// A request the responder got: the operation, the HTTP path, its attributes as "name=value,value" and the
/// document after the attributes.
#[derive(Clone, Debug)]
struct Got {
    op: u16,
    path: String,
    attrs: Vec<String>,
    doc: Vec<u8>,
}

impl Got {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find_map(|a| a.strip_prefix(&format!("{name}=")))
    }
}

fn decode(body: &[u8]) -> (u16, u32, Vec<String>, Vec<u8>) {
    let op = u16::from_be_bytes([body[2], body[3]]);
    let id = u32::from_be_bytes([body[4], body[5], body[6], body[7]]);
    let mut attrs: Vec<String> = Vec::new();
    let mut i = 8;
    while i < body.len() {
        let tag = body[i];
        i += 1;
        if tag < 0x10 {
            if tag == 0x03 {
                break;
            }
            continue;
        }
        let n = u16::from_be_bytes([body[i], body[i + 1]]) as usize;
        let name = String::from_utf8_lossy(&body[i + 2..i + 2 + n]).into_owned();
        i += 2 + n;
        let m = u16::from_be_bytes([body[i], body[i + 1]]) as usize;
        let raw = &body[i + 2..i + 2 + m];
        i += 2 + m;
        let value = match tag {
            0x21 | 0x23 => i32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]).to_string(),
            0x22 => (raw[0] != 0).to_string(),
            _ => String::from_utf8_lossy(raw).into_owned(),
        };
        if n == 0 {
            if let Some(last) = attrs.last_mut() {
                last.push(',');
                last.push_str(&value);
            }
        } else {
            attrs.push(format!("{name}={value}"));
        }
    }
    (op, id, attrs, body[i.min(body.len())..].to_vec())
}

type Answer = Box<dyn Fn(&Got, u32) -> Vec<u8> + Send + Sync>;

/// An HTTP/1.1 IPP server on 127.0.0.1 for libcups: keep-alive, `Expect: 100-continue`, chunked bodies, `GET
/// /printers/NAME.ppd`; every IPP request is recorded.
struct Responder {
    port: u16,
    got: Arc<Mutex<Vec<Got>>>,
}

fn read_body(reader: &mut BufReader<TcpStream>, headers: &str) -> Vec<u8> {
    let lower = headers.to_ascii_lowercase();
    if lower.contains("transfer-encoding: chunked") {
        let mut out = Vec::new();
        loop {
            let mut size = String::new();
            reader.read_line(&mut size).unwrap();
            let n = usize::from_str_radix(size.trim().split(';').next().unwrap_or("0"), 16)
                .unwrap_or(0);
            if n == 0 {
                let mut end = String::new();
                reader.read_line(&mut end).unwrap();
                return out;
            }
            let mut chunk = vec![0; n];
            reader.read_exact(&mut chunk).unwrap();
            out.extend_from_slice(&chunk);
            let mut crlf = String::new();
            reader.read_line(&mut crlf).unwrap();
        }
    }
    let n = lower
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; n];
    reader.read_exact(&mut body).unwrap();
    body
}

impl Responder {
    fn start(answer: Answer, ppd: &'static str) -> Responder {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let got: Arc<Mutex<Vec<Got>>> = Arc::default();
        let (seen, answer) = (got.clone(), Arc::new(answer));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let (seen, answer) = (seen.clone(), answer.clone());
                std::thread::spawn(move || {
                    let mut writer = stream.try_clone().unwrap();
                    let mut reader = BufReader::new(stream);
                    loop {
                        let mut head = String::new();
                        loop {
                            let mut line = String::new();
                            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                                return;
                            }
                            if line == "\r\n" || line == "\n" {
                                break;
                            }
                            head.push_str(&line);
                        }
                        let path = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
                        if head.starts_with("GET ") {
                            let body = if path.ends_with(".ppd") {
                                ppd.as_bytes()
                            } else {
                                b"" as &[u8]
                            };
                            let status = if body.is_empty() {
                                "404 Not Found"
                            } else {
                                "200 OK"
                            };
                            let _ = write!(
                                writer,
                                "HTTP/1.1 {status}\r\nContent-Type: application/vnd.cups-ppd\r\nContent-Length: {}\r\n\r\n",
                                body.len()
                            );
                            let _ = writer.write_all(body);
                            continue;
                        }
                        if head.to_ascii_lowercase().contains("expect: 100-continue") {
                            let _ = writer.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
                        }
                        let body = read_body(&mut reader, &head);
                        let (op, id, attrs, doc) = decode(&body);
                        let request = Got {
                            op,
                            path,
                            attrs,
                            doc,
                        };
                        let out = answer(&request, id);
                        seen.lock().unwrap().push(request);
                        let _ = write!(
                            writer,
                            "HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\n\r\n",
                            out.len()
                        );
                        let _ = writer.write_all(&out);
                    }
                });
            }
        });
        Responder { port, got }
    }

    fn got(&self, op: u16) -> Vec<Got> {
        self.got
            .lock()
            .unwrap()
            .iter()
            .filter(|g| g.op == op)
            .cloned()
            .collect()
    }
}

const FAKE_PPD: &str = "*PPD-Adobe: \"4.3\"\n*OpenGroup: General/General\n*OpenUI *PageSize/Media Size: PickOne\n*DefaultPageSize: Letter\n*PageSize A4/A4: \"\"\n*PageSize Letter/US Letter: \"\"\n*CloseUI: *PageSize\n*CloseGroup: General\n";

fn scheduler(port_of: Arc<Mutex<u16>>) -> Answer {
    Box::new(move |req, id| {
        let port = *port_of.lock().unwrap();
        let queue = |name: &str, info: &str, state: i32| -> Vec<(&'static str, Vec<V>)> {
            vec![
                ("printer-name", vec![s(0x42, name)]),
                ("printer-info", vec![s(0x41, info)]),
                ("printer-location", vec![s(0x41, "Kantoor")]),
                (
                    "printer-make-and-model",
                    vec![s(0x41, "HP LaserJet Pro M404-M405, hpcups 3.23")],
                ),
                ("printer-state", vec![V::E(state)]),
                ("printer-state-reasons", vec![s(0x44, "none")]),
                ("printer-is-accepting-jobs", vec![V::B(true)]),
                ("printer-type", vec![V::E(0x0089D04C)]),
                ("device-uri", vec![s(0x45, "ipp://192.168.1.20/ipp/print")]),
                (
                    "printer-uri-supported",
                    vec![V::S(
                        0x45,
                        format!("ipp://127.0.0.1:{port}/printers/{name}"),
                    )],
                ),
                ("marker-names", vec![s(0x42, "Black Toner")]),
                ("marker-levels", vec![V::I(64)]),
                (
                    "printer-commands",
                    vec![s(0x44, "AutoConfigure"), s(0x44, "Clean")],
                ),
                ("PageSize-default", vec![s(0x42, "A4")]),
                ("number-up-supported", vec![V::I(1), V::I(2), V::I(4)]),
                ("number-up-default", vec![V::I(1)]),
                (
                    "sides-supported",
                    vec![s(0x44, "one-sided"), s(0x44, "two-sided-long-edge")],
                ),
            ]
        };
        match req.op {
            0x4002 => encode(
                0,
                id,
                &[
                    (0x04, queue("HP_LaserJet", "HP LaserJet", 3)),
                    (0x04, queue("Epson", "Epson ET-2850", 5)),
                ],
            ),
            0x4001 => encode(
                0,
                id,
                &[(0x04, vec![("printer-name", vec![s(0x42, "Epson")])])],
            ),
            0x000B
                if req
                    .attr("printer-uri")
                    .is_some_and(|u| u.ends_with("/Gone")) =>
            {
                encode(0x0406, id, &[])
            }
            0x000B => encode(0, id, &[(0x04, queue("HP_LaserJet", "HP LaserJet", 3))]),
            0x000A => encode(
                0,
                id,
                &[
                    (
                        0x02,
                        vec![
                            ("job-id", vec![V::I(41)]),
                            ("job-name", vec![s(0x42, "Rapport.pdf")]),
                            ("job-state", vec![V::E(5)]),
                            (
                                "job-printer-uri",
                                vec![s(0x45, "ipp://localhost/printers/HP_LaserJet")],
                            ),
                            ("job-originating-user-name", vec![s(0x42, "robin")]),
                        ],
                    ),
                    (
                        0x02,
                        vec![
                            ("job-id", vec![V::I(42)]),
                            ("job-name", vec![s(0x42, "Foto.jpg")]),
                            ("job-state", vec![V::E(4)]),
                            (
                                "job-printer-uri",
                                vec![s(0x45, "ipp://localhost/printers/HP_LaserJet")],
                            ),
                        ],
                    ),
                ],
            ),
            0x0002 => encode(
                0,
                id,
                &[(
                    0x02,
                    vec![("job-id", vec![V::I(100)]), ("job-state", vec![V::E(3)])],
                )],
            ),
            0x0014 if req.attr("job-uri").is_some_and(|u| u.ends_with("/99")) => {
                encode(0x0406, id, &[])
            }
            0x0014 => encode(0, id, &[]),
            0x400C => encode(
                0,
                id,
                &[
                    (
                        0x04,
                        vec![
                            ("ppd-name", vec![s(0x42, "everywhere")]),
                            ("ppd-make", vec![s(0x41, "Generic")]),
                            ("ppd-make-and-model", vec![s(0x41, "IPP Everywhere")]),
                        ],
                    ),
                    (
                        0x04,
                        vec![
                            ("ppd-name", vec![s(0x42, "hp-m404.ppd")]),
                            ("ppd-make", vec![s(0x41, "HP")]),
                            (
                                "ppd-make-and-model",
                                vec![s(0x41, "HP LaserJet Pro M404-M405, hpcups 3.23")],
                            ),
                            (
                                "ppd-device-id",
                                vec![s(0x41, "MFG:HP;MDL:HP LaserJet Pro M404-M405;")],
                            ),
                        ],
                    ),
                ],
            ),
            // the watcher's subscription: Create-Printer-Subscriptions, Renew-Subscription, Cancel-Subscription
            0x0016 => encode(
                0,
                id,
                &[(0x06, vec![("notify-subscription-id", vec![V::I(7)])])],
            ),
            0x001A | 0x001B => encode(0, id, &[]),
            _ => encode(0x0501, id, &[]),
        }
    })
}

/// Printers with a private scheduler on loopback, the helper `helper` on the session bus, and a test page.
fn service(port: u16, helper: &str, lc: &LangContext) -> Arc<Printers> {
    let test_page = lc.root.join("testprint");
    std::fs::write(&test_page, "#PDF-BANNER\nTemplate default-testpage.pdf\n").unwrap();
    Printers::with(
        lc.ctx.clone(),
        Endpoints {
            system_bus: false,
            helper: helper.into(),
            avahi: "org.example.NoAvahi".into(),
            server: Server {
                address: Some(("127.0.0.1".into(), port)),
            },
            lpoptions: Some(lc.root.join("lpoptions")),
            test_page,
        },
    )
}

fn libcups() -> bool {
    if cups::lib().is_none() {
        eprintln!("skipped: libcups.so.2 is not installed");
        return false;
    }
    true
}

#[test]
fn libcups_reads_the_queues_their_options_and_jobs() {
    if !libcups() {
        return;
    }
    let port = Arc::new(Mutex::new(0));
    let ipp = Responder::start(scheduler(port.clone()), FAKE_PPD);
    *port.lock().unwrap() = ipp.port;
    let lc = LangContext::new("en", "settings");
    let printers = service(ipp.port, "org.example.NoHelper", &lc);

    let list = printers.call("printers.list", &json!({})).unwrap().unwrap();
    assert_eq!(list["available"], true);
    assert_eq!(list["default"], "Epson", "CUPS-Get-Default");
    let titles: Vec<&str> = list["printers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["Epson ET-2850", "HP LaserJet"]);
    assert_eq!(list["printers"][1]["canClean"], true);
    assert_eq!(list["printers"][0]["state"], "stopped");
    let asked = &ipp.got(0x4002)[0];
    assert!(
        asked
            .attr("requested-attributes")
            .is_some_and(|a| a.contains("marker-levels")),
        "{asked:?}"
    );
    assert!(asked.attr("requesting-user-name").is_some());
    // the user's own default wins
    std::fs::write(lc.root.join("lpoptions"), "Default HP_LaserJet\n").unwrap();
    let list = printers.call("printers.list", &json!({})).unwrap().unwrap();
    assert_eq!(list["default"], "HP_LaserJet");

    let got = printers
        .call("printers.get", &json!({"name": "HP_LaserJet"}))
        .unwrap()
        .unwrap();
    assert_eq!(got["ppd"], true, "the PPD came over HTTP: {got}");
    let size = &got["options"][0]["options"][0];
    assert_eq!(
        (size["name"].clone(), size["value"].clone()),
        (json!("PageSize"), json!("A4")),
        "PageSize-default beats the PPD's Letter"
    );
    let extra: Vec<&str> = got["ipp"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        extra,
        ["number-up", "sides"],
        "sides because the PPD has no Duplex"
    );
    let gone = printers
        .call("printers.get", &json!({"name": "Gone"}))
        .unwrap()
        .unwrap_err();
    assert_eq!(gone.key.as_deref(), Some("printers.notFound"));

    let jobs = printers
        .call("printers.jobs", &json!({"name": "HP_LaserJet"}))
        .unwrap()
        .unwrap();
    let jobs = jobs["jobs"].as_array().unwrap();
    assert_eq!((jobs.len(), jobs[1]["held"].clone()), (2, json!(true)));
    let asked = ipp.got(0x000A);
    assert_eq!(asked[0].attr("which-jobs"), Some("not-completed"));
    assert_eq!(
        asked[0].attr("my-jobs"),
        Some("false"),
        "every user's jobs, as GNOME's dialog"
    );

    let drivers = printers
        .call(
            "printers.drivers",
            &json!({"deviceId": "MFG:HP;MDL:HP LaserJet Pro M404-M405;"}),
        )
        .unwrap()
        .unwrap();
    assert_eq!(drivers["everywhere"], true);
    assert_eq!(drivers["best"][0]["name"], "hp-m404.ppd");
}

#[test]
fn the_test_page_and_the_head_cleaning_are_print_jobs_of_the_user() {
    if !libcups() {
        return;
    }
    let port = Arc::new(Mutex::new(0));
    let ipp = Responder::start(scheduler(port.clone()), FAKE_PPD);
    *port.lock().unwrap() = ipp.port;
    let lc = LangContext::new("en", "settings");
    let printers = service(ipp.port, "org.example.NoHelper", &lc);
    let r = printers
        .call("printers.testPage", &json!({"name": "HP_LaserJet"}))
        .unwrap()
        .unwrap();
    assert_eq!(r["job"], 100);
    let job = &ipp.got(0x0002)[0];
    assert_eq!(
        job.path, "/printers/HP_LaserJet",
        "Print-Job goes to the queue's resource"
    );
    assert_eq!(job.attr("job-name"), Some("Test page"));
    assert!(
        job.attr("printer-uri")
            .unwrap()
            .ends_with("/printers/HP_LaserJet")
    );
    assert!(
        job.doc.starts_with(b"#PDF-BANNER"),
        "{:?}",
        String::from_utf8_lossy(&job.doc)
    );
    printers
        .call("printers.cleanHeads", &json!({"name": "HP_LaserJet"}))
        .unwrap()
        .unwrap();
    let job = ipp.got(0x0002).pop().unwrap();
    assert_eq!(
        job.attr("document-format"),
        Some("application/vnd.cups-command")
    );
    assert_eq!(job.doc, b"#CUPS-COMMAND\nClean all\n");
    printers
        .call("printers.job.priority", &json!({"id": 41, "priority": 100}))
        .unwrap()
        .unwrap();
    let set = &ipp.got(0x0014)[0];
    assert_eq!(
        (set.attr("job-uri"), set.attr("job-priority")),
        (Some("ipp://localhost/jobs/41"), Some("100"))
    );
    let gone = printers
        .call("printers.job.priority", &json!({"id": 99, "priority": 100}))
        .unwrap()
        .unwrap_err();
    assert_eq!(gone.key.as_deref(), Some("printers.jobNotFound"));
}

#[test]
fn no_scheduler_is_gnomes_no_print_service() {
    if !libcups() {
        return;
    }
    // a port nobody listens on
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let lc = LangContext::new("en", "settings");
    let printers = service(port, "org.example.NoHelper", &lc);
    let list = printers.call("printers.list", &json!({})).unwrap().unwrap();
    assert_eq!(
        (list["available"].clone(), list["reason"].clone()),
        (json!(false), json!("cupsd"))
    );
    let err = printers
        .call("printers.jobs", &json!({}))
        .unwrap()
        .unwrap_err();
    assert_eq!(err.key.as_deref(), Some("printers.noCups"));
}

// ------------------------------------------------------------------------------- cups-pk-helper's fake

const HELPER_XML: &str = r#"<node><interface name="org.opensuse.CupsPkHelper.Mechanism">
  <method name="PrinterAdd"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="out"/></method>
  <method name="PrinterAddWithPpdFile"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="out"/></method>
  <method name="PrinterSetEnabled"><arg type="s" direction="in"/><arg type="b" direction="in"/><arg type="s" direction="out"/></method>
  <method name="PrinterSetAcceptJobs"><arg type="s" direction="in"/><arg type="b" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="out"/></method>
  <method name="PrinterAddOptionDefault"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="as" direction="in"/><arg type="s" direction="out"/></method>
  <method name="PrinterSetDefault"><arg type="s" direction="in"/><arg type="s" direction="out"/></method>
  <method name="PrinterDelete"><arg type="s" direction="in"/><arg type="s" direction="out"/></method>
  <method name="PrinterRename"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="out"/></method>
  <method name="PrinterSetLocation"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="out"/></method>
  <method name="JobCancelPurge"><arg type="i" direction="in"/><arg type="b" direction="in"/><arg type="s" direction="out"/></method>
  <method name="JobSetHoldUntil"><arg type="i" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="out"/></method>
  <method name="DevicesGet"><arg type="i" direction="in"/><arg type="i" direction="in"/><arg type="as" direction="in"/><arg type="as" direction="in"/><arg type="s" direction="out"/><arg type="a{ss}" direction="out"/></method>
</interface></node>"#;

fn helper_fake() -> Option<Fake> {
    let handler: Handler = Arc::new(|member: &str, args: &glib::Variant, _: &Props| {
        let first = args
            .child_value(0)
            .str()
            .map(str::to_owned)
            .unwrap_or_default();
        // "deny" as a queue, or job 41 (another user's: job-not-owned-edit) for a cancel
        if first == "deny"
            || (member == "JobCancelPurge" && args.child_value(0).get::<i32>() == Some(41))
        {
            return Err((
                "org.opensuse.CupsPkHelper.Mechanism.NotPrivileged".into(),
                "Not authorized".into(),
            ));
        }
        if member == "DevicesGet" {
            let mut map = std::collections::BTreeMap::new();
            map.insert(
                "device-uri:0".to_owned(),
                "usb://HP/DeskJet?serial=1".to_owned(),
            );
            map.insert("device-class:0".to_owned(), "direct".to_owned());
            map.insert("device-info:0".to_owned(), "HP DeskJet".to_owned());
            return Ok(Some(("", map).to_variant()));
        }
        let error = if first == "refuse" {
            "client-error-not-possible"
        } else {
            ""
        };
        Ok(Some((error,).to_variant()))
    });
    Fake::start(
        vec![Object {
            path: "/".into(),
            xml: HELPER_XML.into(),
            handler,
        }],
        vec![],
    )
}

fn skipped() {
    eprintln!("skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)");
}

#[test]
fn every_admin_write_goes_through_cups_pk_helper_with_the_password_dialog() {
    let Some(fake) = helper_fake() else {
        return skipped();
    };
    if !libcups() {
        return;
    }
    let port = Arc::new(Mutex::new(0));
    let ipp = Responder::start(scheduler(port.clone()), FAKE_PPD);
    *port.lock().unwrap() = ipp.port;
    let lc = LangContext::new("en", "settings");
    let printers = service(ipp.port, &fake.name, &lc);
    let call = |m: &str, a: Value| printers.call(m, &a).unwrap();

    // a new driverless queue: PrinterAdd with "everywhere", then enabled, accepting jobs and A4 (amendment 41)
    let r = call(
        "printers.add",
        json!({"name": "Gone", "uri": "ipp://127.0.0.1:631/ipp/print", "ppd": "everywhere",
        "info": "Fake LaserJet", "location": "Lab"}),
    )
    .unwrap();
    assert_eq!(r["new"], true);
    let members: Vec<String> = fake
        .calls
        .lock()
        .unwrap()
        .iter()
        .map(|c| c.member.clone())
        .collect();
    assert_eq!(
        members,
        [
            "PrinterAdd",
            "PrinterSetEnabled",
            "PrinterSetAcceptJobs",
            "PrinterAddOptionDefault"
        ]
    );
    assert_eq!(
        fake.calls_of("PrinterAdd")[0].args,
        "('Gone', 'ipp://127.0.0.1:631/ipp/print', 'everywhere', 'Fake LaserJet', 'Lab')"
    );
    assert_eq!(
        fake.calls_of("PrinterSetAcceptJobs")[0].args,
        "('Gone', true, '')"
    );
    assert_eq!(
        fake.calls_of("PrinterAddOptionDefault")[0].args,
        "('Gone', 'media', ['a4'])"
    );
    assert!(
        fake.calls.lock().unwrap().iter().all(|c| c.interactive),
        "every write may ask for the password"
    );

    // a new driver for an existing queue only replaces the driver
    fake.clear();
    let ppd = lc.root.join("driver.ppd");
    std::fs::write(&ppd, FAKE_PPD).unwrap();
    let r = call(
        "printers.add",
        json!({"name": "HP_LaserJet", "uri": "ipp://192.168.1.20/ipp/print",
        "ppdFile": ppd.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(r["new"], false);
    let members: Vec<String> = fake
        .calls
        .lock()
        .unwrap()
        .iter()
        .map(|c| c.member.clone())
        .collect();
    assert_eq!(members, ["PrinterAddWithPpdFile"]);

    fake.clear();
    std::fs::write(
        lc.root.join("lpoptions"),
        "Default Epson\nDest HP_LaserJet sides=one-sided\n",
    )
    .unwrap();
    for (method, args, member, expect) in [
        (
            "printers.setDefault",
            json!({"name": "HP_LaserJet"}),
            "PrinterSetDefault",
            "('HP_LaserJet',)",
        ),
        (
            "printers.rename",
            json!({"name": "HP_LaserJet", "newName": "Laser"}),
            "PrinterRename",
            "('HP_LaserJet', 'Laser')",
        ),
        (
            "printers.setLocation",
            json!({"name": "Laser", "location": "Zolder"}),
            "PrinterSetLocation",
            "('Laser', 'Zolder')",
        ),
        (
            "printers.setEnabled",
            json!({"name": "Laser", "on": true}),
            "PrinterSetEnabled",
            "('Laser', true)",
        ),
        (
            "printers.setAcceptJobs",
            json!({"name": "Laser", "on": false}),
            "PrinterSetAcceptJobs",
            "('Laser', false, '')",
        ),
        (
            "printers.setOption",
            json!({"name": "Laser", "option": "PageSize", "values": ["A4"]}),
            "PrinterAddOptionDefault",
            "('Laser', 'PageSize', ['A4'])",
        ),
        (
            "printers.setOption",
            json!({"name": "Laser", "option": "number-up", "values": 2}),
            "PrinterAddOptionDefault",
            "('Laser', 'number-up', ['2'])",
        ),
        (
            "printers.remove",
            json!({"name": "Laser"}),
            "PrinterDelete",
            "('Laser',)",
        ),
        (
            "printers.job.cancel",
            json!({"id": 42}),
            "JobCancelPurge",
            "(42, false)",
        ),
        (
            "printers.job.hold",
            json!({"id": 41}),
            "JobSetHoldUntil",
            "(41, 'indefinite')",
        ),
        (
            "printers.job.release",
            json!({"id": 41}),
            "JobSetHoldUntil",
            "(41, 'no-hold')",
        ),
    ] {
        call(method, args).unwrap_or_else(|e| panic!("{method}: {e:?}"));
        let last = fake.calls.lock().unwrap().last().cloned().unwrap();
        assert_eq!(
            (last.member.as_str(), last.args.as_str(), last.interactive),
            (member, expect, true),
            "{method}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(lc.root.join("lpoptions")).unwrap(),
        "Dest Epson\nDest HP_LaserJet sides=one-sided\n",
        "this computer's scheduler: the user's own default went (g-c-c's set_local_default_printer(NULL))"
    );

    // Alles annuleren: every job of the queue on its own, job 41's refusal keeps job 42 from nothing
    fake.clear();
    let r = call("printers.jobs.clear", json!({"name": "HP_LaserJet"})).unwrap();
    assert_eq!(
        (r["canceled"].clone(), r["failed"].clone()),
        (json!(1), json!(1))
    );
    let cancels: Vec<String> = fake
        .calls_of("JobCancelPurge")
        .iter()
        .map(|c| c.args.clone())
        .collect();
    assert_eq!(cancels, ["(41, false)", "(42, false)"]);

    // another computer's scheduler: only the user's own default (a new ~/.cups/lpoptions), no PrinterSetDefault
    fake.clear();
    let remote_home = lc.root.join("remote-home/.cups/lpoptions");
    let remote = Printers::with(
        lc.ctx.clone(),
        Endpoints {
            system_bus: false,
            helper: fake.name.clone(),
            avahi: "org.example.NoAvahi".into(),
            server: Server {
                address: Some(("print.example.invalid".into(), 631)),
            },
            lpoptions: Some(remote_home.clone()),
            test_page: lc.root.join("testprint"),
        },
    );
    remote
        .call("printers.setDefault", &json!({"name": "Kantoor"}))
        .unwrap()
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&remote_home).unwrap(),
        "Default Kantoor\n"
    );
    assert!(fake.calls_of("PrinterSetDefault").is_empty());

    // polkit's no, CUPS's refusal
    let denied = call("printers.setDefault", json!({"name": "deny"})).unwrap_err();
    assert_eq!(
        (denied.code.as_str(), denied.key.as_deref()),
        ("denied", Some("core.notAuthorized"))
    );
    let refused = call("printers.remove", json!({"name": "refuse"})).unwrap_err();
    assert_eq!(refused.key.as_deref(), Some("printers.failed"));
    assert!(
        refused.message.contains("client-error-not-possible"),
        "{}",
        refused.message
    );

    // the scheduler's backends (no Avahi here)
    fake.clear();
    let found = call("printers.discover", json!({})).unwrap();
    assert_eq!(found["devices"][0]["uri"], "usb://HP/DeskJet?serial=1");
    let get = &fake.calls_of("DevicesGet")[0];
    assert_eq!(
        (get.args.as_str(), get.interactive),
        ("(10, 0, @as [], ['dnssd'])", true)
    );
    // a typed address: an IPP Everywhere printer answers on /ipp/print
    let found = call(
        "printers.discover",
        json!({"address": format!("127.0.0.1:{}", ipp.port)}),
    )
    .unwrap();
    assert!(
        found["devices"].as_array().unwrap().iter().any(|d| d["uri"]
            .as_str()
            .is_some_and(|u| u.ends_with("/printers/HP_LaserJet"))),
        "the server's queues: {found}"
    );
}

#[test]
fn the_watcher_holds_gnomes_subscription_and_sleeps_between_bursts() {
    // review B findings 2 and 3: a lease of 600 s renewed (Renew-Subscription, 0x001A) on a repeating source, one
    // printers.changed per burst of the scheduler's signals, and shutdown cancels the subscription at once
    if !libcups() {
        return;
    }
    let Some(address) = crate::services::devices::fake_bus::private_bus_address() else {
        return skipped();
    };
    let daemon = gio::DBusConnection::for_address_sync(
        &address,
        gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
            | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
        None,
        gio::Cancellable::NONE,
    )
    .unwrap();
    let port = Arc::new(Mutex::new(0));
    let ipp = Responder::start(scheduler(port.clone()), FAKE_PPD);
    *port.lock().unwrap() = ipp.port;
    let lc = LangContext::new("en", "settings");
    let seen: Arc<Mutex<usize>> = Arc::default();
    let sink = seen.clone();
    lc.ctx.events.on(
        "printers.changed",
        Arc::new(move |_: &Value| *sink.lock().unwrap() += 1),
    );
    let printers = Printers::with_renewal(
        lc.ctx.clone(),
        Endpoints {
            system_bus: false,
            helper: "org.example.NoHelper".into(),
            avahi: "org.example.NoAvahi".into(),
            server: Server {
                address: Some(("127.0.0.1".into(), ipp.port)),
            },
            lpoptions: None,
            test_page: lc.root.join("testprint"),
        },
        Duration::from_millis(300),
    );
    printers.watch();
    let wait = |what: &str, done: &dyn Fn() -> bool| {
        let end = std::time::Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(
                std::time::Instant::now() < end,
                "timed out waiting for {what}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    wait("the subscription", &|| !ipp.got(0x0016).is_empty());
    let create = &ipp.got(0x0016)[0];
    assert_eq!(create.attr("notify-lease-duration"), Some("600"));
    assert_eq!(create.attr("notify-recipient-uri"), Some("dbus://"));
    assert!(
        create
            .attr("notify-events")
            .is_some_and(|e| e.contains("printer-added") && e.contains("job-completed"))
    );
    wait("a renewal", &|| !ipp.got(0x001A).is_empty());
    let renew = &ipp.got(0x001A)[0];
    assert_eq!(
        (
            renew.attr("notify-subscription-id"),
            renew.attr("notify-lease-duration")
        ),
        (Some("7"), Some("600"))
    );
    assert_eq!(ipp.got(0x0016).len(), 1, "renewed, not made again");

    // a burst of the scheduler's signals is one event
    for member in ["PrinterStateChanged", "JobCreated", "JobState"] {
        daemon
            .emit_signal(
                None,
                "/org/cups/cupsd/Notifier",
                "org.cups.cupsd.Notifier",
                member,
                Some(&("text",).to_variant()),
            )
            .unwrap();
    }
    daemon.flush_sync(gio::Cancellable::NONE).unwrap();
    wait("the event", &|| *seen.lock().unwrap() > 0);
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(*seen.lock().unwrap(), 1);

    printers.shutdown();
    let cancel = ipp.got(0x001B);
    assert_eq!(cancel.len(), 1, "cancelled by shutdown itself");
    assert_eq!(cancel[0].attr("notify-subscription-id"), Some("7"));
    let renewals = ipp.got(0x001A).len();
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(ipp.got(0x001A).len(), renewals, "no renewal after shutdown");
    assert_eq!(ipp.got(0x001B).len(), 1);
}

#[test]
fn a_dry_run_writes_nothing() {
    let Some(fake) = helper_fake() else {
        return skipped();
    };
    let lc = LangContext::new("en", "settings");
    let events = crate::events::Events::new(&lc.root.join("events-dry")).unwrap();
    let ctx = ServiceContext::new(
        events.clone(),
        lc.root.join("config"),
        lc.root.join("cache"),
        lc.root.clone(),
        "settings".into(),
        true,
    )
    .unwrap();
    let ipp = Responder::start(Box::new(|_, id| encode(0, id, &[])), FAKE_PPD);
    let printers = Printers::with(
        ctx,
        Endpoints {
            system_bus: false,
            helper: fake.name.clone(),
            avahi: "org.example.NoAvahi".into(),
            server: Server {
                address: Some(("127.0.0.1".into(), ipp.port)),
            },
            lpoptions: None,
            test_page: lc.root.join("testprint"),
        },
    );
    for (method, args) in [
        (
            "printers.add",
            json!({"name": "HP", "uri": "ipp://h/ipp/print", "ppd": "everywhere"}),
        ),
        ("printers.rename", json!({"name": "HP", "newName": "HP2"})),
        (
            "printers.setLocation",
            json!({"name": "HP", "location": ""}),
        ),
        ("printers.setDefault", json!({"name": "HP"})),
        ("printers.setEnabled", json!({"name": "HP", "on": true})),
        ("printers.setAcceptJobs", json!({"name": "HP", "on": true})),
        ("printers.remove", json!({"name": "HP"})),
        (
            "printers.setOption",
            json!({"name": "HP", "option": "PageSize", "values": ["A4"]}),
        ),
        ("printers.testPage", json!({"name": "HP"})),
        ("printers.cleanHeads", json!({"name": "HP"})),
        ("printers.job.cancel", json!({"id": 1})),
        ("printers.job.hold", json!({"id": 1})),
        ("printers.job.release", json!({"id": 1})),
        ("printers.job.priority", json!({"id": 1, "priority": 100})),
        (
            "printers.job.authenticate",
            json!({"id": 1, "username": "u", "password": "p"}),
        ),
        ("printers.jobs.clear", json!({"name": "HP"})),
    ] {
        let r = printers.call(method, &args).unwrap().unwrap();
        assert_eq!(r["dryRun"], true, "{method}");
    }
    assert!(
        fake.calls.lock().unwrap().is_empty(),
        "no helper call: {:?}",
        fake.calls.lock().unwrap()
    );
    assert!(ipp.got.lock().unwrap().is_empty(), "no scheduler request");
    let bad = printers
        .call("printers.remove", &json!({"name": "a b"}))
        .unwrap()
        .unwrap_err();
    assert_eq!(
        bad.key.as_deref(),
        Some("printers.badName"),
        "the arguments are checked in a dry run too"
    );
    events.shutdown();
}

#[test]
fn every_error_key_is_a_row_of_printers_tsv() {
    for key in [
        "printers.badArgs",
        "printers.badName",
        "printers.badAddress",
        "printers.badFile",
        "printers.noCups",
        "printers.noLibcups",
        "printers.noHelper",
        "printers.notFound",
        "printers.jobNotFound",
        "printers.failed",
        "printers.noTestPage",
        "printers.noCleaning",
        "printers.testPageTitle",
        "printers.cleanHeadsTitle",
    ] {
        for lang in [Lang::Nl, Lang::En] {
            let text = crate::i18n::tr(lang, key);
            assert!(!text.is_empty() && text != key, "{key} ({lang:?})");
        }
    }
}
