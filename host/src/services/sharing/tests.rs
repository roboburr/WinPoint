//! sharing.* tests: grdctl's output and the password that never reaches its command line, the GnuTLS certificate
//! and its fingerprint, rygel.conf as GNOME writes it, the SSH units' state, the allow rows, dry runs that touch
//! nothing, the texts, and (`#[ignore]`, only inside tests/dbus-fakes/run.sh --batch c with fakes/c/sharing.py) the
//! calls to systemd, the RDP configuration daemon, gsd Sharing and PackageKit.
use super::*;
use crate::services::system::common::LangContext;
use std::os::unix::fs::PermissionsExt;

/// What `grdctl status --show-credentials` printed in a private keyring (dbus-fakes layer, gnome-remote-desktop 50.2).
const STATUS: &str = "RDP server certificate is invalid.\nRDP:\n\tStatus: disabled\n\tPort: 3389\n\tAuthentication methods: credentials\n\tTLS certificate: \n\tTLS fingerprint: (null)\n\tTLS key: \n\tKerberos keytab: \n\tView-only: yes\n\tNegotiate port: yes\n\tUsername: alice\n\tPassword: p@ss w0rd;x\n";

/// A private temporary directory, removed when dropped.
struct Dir(std::path::PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let d =
            std::env::temp_dir().join(format!("win11-sharing-{tag}-{}", crate::events::nonce()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700)).unwrap();
        Dir(d)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A grdctl stand-in: every argv as a line of `argv`, every standard input of set-credentials as a line of `stdin`,
/// `status` answers from `status` (set-credentials writes it), `fail` makes every call fail.
fn fake_grdctl(dir: &Path) -> PathBuf {
    let tool = dir.join("grdctl");
    std::fs::write(
        &tool,
        r#"#!/bin/sh
d="$(dirname "$0")"
printf '%s\n' "$*" >> "$d/argv"
if [ -e "$d/fail" ]; then echo "[12:00:00:000] [ERROR][com.freerdp] - noise" >&2; echo "Failed to store credentials" >&2; exit 1; fi
case "$1" in
  status) cat "$d/status" 2>/dev/null; exit 0 ;;
  rdp) IFS= read -r pw; printf '%s\n' "$pw" >> "$d/stdin"
       printf 'RDP:\n\tStatus: enabled\n\tUsername: %s\n\tPassword: %s\n' "$3" "$pw" > "$d/status"; exit 0 ;;
esac
exit 2
"#,
    )
    .unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o700)).unwrap();
    tool
}

fn lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn grdctl_status_gives_the_rdp_credentials() {
    assert_eq!(
        rdp::parse_status(STATUS),
        (Some("alice".into()), Some("p@ss w0rd;x".into()))
    );
    let hidden = "RDP:\n\tUsername: (hidden)\n\tPassword: (hidden)\n";
    assert_eq!(rdp::parse_status(hidden), (None, None));
    let cleared = "RDP:\n\tUsername: (null)\n\tPassword: (null)\nVNC:\n\tPassword: vnc-secret\n";
    assert_eq!(
        rdp::parse_status(cleared),
        (None, None),
        "the VNC section's password is not RDP's"
    );
    assert_eq!(
        rdp::parse_status("Failed to read credentials\nRDP:\n\tStatus: disabled\n"),
        (None, None)
    );
}

#[test]
fn grdctl_gets_the_password_on_its_standard_input_only() {
    let lang = Lang::En;
    let dir = Dir::new("grdctl");
    let tool = fake_grdctl(&dir.0);
    assert_eq!(
        rdp::credentials(Some(&tool), lang).unwrap(),
        (None, None),
        "nothing stored yet"
    );
    rdp::set_credentials(Some(&tool), "alice", "Geheim 12;x", lang).unwrap();
    assert_eq!(
        lines(&dir.0.join("argv")),
        ["status --show-credentials", "rdp set-credentials alice"]
    );
    assert_eq!(lines(&dir.0.join("stdin")), ["Geheim 12;x"]);
    assert_eq!(
        rdp::credentials(Some(&tool), lang).unwrap(),
        (Some("alice".into()), Some("Geheim 12;x".into()))
    );
    // a failure names grdctl's own last line, never FreeRDP's log noise or the password
    std::fs::write(dir.0.join("fail"), "").unwrap();
    let e = rdp::set_credentials(Some(&tool), "alice", "Geheim 12;x", lang).unwrap_err();
    assert_eq!(e.code, "failed");
    assert_eq!(e.key.as_deref(), Some("sharing.credentialsFailed"));
    let data = format!("{} {}", e.message, e.params.clone().unwrap_or_default());
    assert!(
        data.contains("Failed to store credentials") && !data.contains("Geheim"),
        "{data}"
    );
    // no grdctl: Remote Desktop is not available
    let e = rdp::credentials(Some(&dir.0.join("missing")), lang).unwrap_err();
    assert_eq!(e.code, "unsupported");
}

#[test]
fn credentials_and_generated_passwords_are_what_grdctl_takes() {
    for good in ["alice", "Jan de Vries", "ROBIN"] {
        assert!(rdp::username_ok(good), "{good}");
    }
    for bad in ["", "-x", "a\nb", "a\tb", &"x".repeat(257)] {
        assert!(!rdp::username_ok(bad), "{bad:?}");
    }
    assert!(rdp::password_ok("p@ss w0rd;x") && !rdp::password_ok("") && !rdp::password_ok("a\nb"));
    let (a, b) = (rdp::random_password(), rdp::random_password());
    assert_eq!(a.chars().count(), 16);
    assert!(a.chars().all(|c| c.is_ascii_alphanumeric()) && rdp::password_ok(&a));
    assert_ne!(a, b);
    assert_eq!(
        rdp::normal_fingerprint("ab:cd:ef 01"),
        "AB:CD:EF:01",
        "the system daemon's fingerprint in the page's form"
    );
}

#[test]
fn the_certificate_is_gnomes_and_its_fingerprint_is_sha256() {
    // SHA-256 of "abc" (FIPS 180-2)
    assert_eq!(
        tls::fingerprint(b"abc"),
        "BA:78:16:BF:8F:01:CF:EA:41:41:40:DE:5D:AE:22:23:B0:03:61:A3:96:17:7A:9C:B4:10:FF:61:F2:00:15:AD"
    );
    assert_eq!(
        tls::pem_der("x\n-----BEGIN CERTIFICATE-----\nYWJj\n-----END CERTIFICATE-----\n"),
        Some(b"abc".to_vec())
    );
    assert_eq!(
        tls::pem_der("-----BEGIN CERTIFICATE-----\n!!\n-----END CERTIFICATE-----"),
        None
    );
    if tls::lib().is_none() {
        eprintln!("skipped: libgnutls.so.30 is not installed");
        return;
    }
    let lang = Lang::En;
    let dir = Dir::new("tls");
    let certs = dir.0.join("gnome-remote-desktop").join("certificates");
    let now = 1_763_300_000;
    // nothing configured: a new pair, in GNOME's files
    let (cert, key) = rdp::ensure_certificate(&certs, "", "", now, lang)
        .unwrap()
        .expect("a new certificate");
    assert_eq!(cert, certs.join("rdp-tls.crt"));
    assert_eq!(key, certs.join("rdp-tls.key"));
    assert!(tls::pair_ok(&cert, &key), "GIO loads it as a TLS identity");
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&key), 0o600, "the private key is the user's alone");
    let pem = std::fs::read_to_string(&cert).unwrap();
    let der = tls::pem_der(&pem).unwrap();
    // the subject C=US, CN=GNOME (printable strings in the DER)
    let has = |needle: &[u8]| der.windows(needle.len()).any(|w| w == needle);
    assert!(has(b"GNOME") && has(b"US"));
    let fp = tls::file_fingerprint(&cert).unwrap();
    assert_eq!(fp.len(), 32 * 3 - 1);
    assert_eq!(fp, tls::fingerprint(&der));
    // a configured pair that loads is kept; a broken one is replaced by the pair in GNOME's place
    let (c, k) = (
        cert.to_string_lossy().into_owned(),
        key.to_string_lossy().into_owned(),
    );
    assert!(
        rdp::ensure_certificate(&certs, &c, &k, now, lang)
            .unwrap()
            .is_none()
    );
    let broken = dir.0.join("broken.crt");
    std::fs::write(&broken, "not a certificate").unwrap();
    assert_eq!(
        rdp::ensure_certificate(&certs, &broken.to_string_lossy(), &k, now, lang).unwrap(),
        Some((cert.clone(), key.clone()))
    );
    assert_eq!(
        tls::file_fingerprint(&cert).unwrap(),
        fp,
        "the good pair stays"
    );
}

#[test]
fn rygel_conf_is_written_as_gnome_writes_it() {
    let specials: Vec<(&str, Option<PathBuf>)> = vec![
        ("@MUSIC@", Some(PathBuf::from("/home/u/Muziek"))),
        ("@VIDEOS@", None),
        ("@PICTURES@", Some(PathBuf::from("/home/u/Afbeeldingen"))),
    ];
    let file = glib::KeyFile::new();
    let flags = glib::KeyFileFlags::KEEP_COMMENTS | glib::KeyFileFlags::KEEP_TRANSLATIONS;
    file.load_from_data(
        "# Rygel's configuration\n[general]\nupnp-enabled=false\n\n[MediaExport]\nenabled=false\nuris=@MUSIC@;@VIDEOS@;/srv/media;\n",
        flags,
    )
    .unwrap();
    assert_eq!(
        media::folders(&file, &specials),
        ["/home/u/Muziek", "/srv/media"],
        "a special folder the user has not got is left out"
    );
    let text = media::with_folders(
        &file,
        &[
            "/home/u/Muziek".into(),
            "/data/a;b".into(),
            "/home/u/Afbeeldingen".into(),
        ],
        &specials,
    );
    assert!(
        text.contains("# Rygel's configuration"),
        "comments kept: {text}"
    );
    assert!(
        text.contains("uris=@MUSIC@;/data/a\\;b;@PICTURES@;"),
        "{text}"
    );
    let back = glib::KeyFile::new();
    back.load_from_data(&text, flags).unwrap();
    assert!(back.boolean("general", "upnp-enabled").unwrap());
    assert!(!back.boolean("Tracker", "enabled").unwrap());
    assert!(!back.boolean("Tracker3", "enabled").unwrap());
    assert!(back.boolean("MediaExport", "enabled").unwrap());
    let uris: Vec<String> = back
        .string_list("MediaExport", "uris")
        .unwrap()
        .iter()
        .map(|s| s.as_str().to_owned())
        .collect();
    assert_eq!(
        uris,
        ["@MUSIC@", "/data/a;b", "@PICTURES@"],
        "GKeyFile reads the escaped list back"
    );
    assert_eq!(media::list_item(" a\\b\tc"), "\\sa\\\\b\\tc");
    let lang = Lang::En;
    assert_eq!(
        media::folders_arg(&json!(["/a", "/a", "/b"]), lang).unwrap(),
        ["/a", "/b"]
    );
    for bad in [
        json!(["relative"]),
        json!(["/a\nb"]),
        json!("/a"),
        json!([1]),
    ] {
        let e = media::folders_arg(&bad, lang).unwrap_err();
        assert_eq!(e.key.as_deref(), Some("sharing.badFolder"), "{bad}");
    }
    assert_eq!(media::status_name(3), "available");
    assert_eq!(media::status_name(1), "disabled-mobile-broadband");
}

#[test]
fn the_ssh_switch_follows_both_units() {
    use systemd::UnitState;
    let unit = |file: &str, active: &str| UnitState {
        exists: true,
        file_state: file.into(),
        active: active.into(),
    };
    let none = UnitState::default();
    let s = ssh::summary(
        &unit("enabled", "active"),
        &unit("disabled", "inactive"),
        false,
        false,
    );
    assert_eq!(
        (
            s["installed"].clone(),
            s["on"].clone(),
            s["enabled"].clone()
        ),
        (json!(true), json!(true), json!(true))
    );
    let s = ssh::summary(
        &unit("disabled", "inactive"),
        &unit("disabled", "inactive"),
        true,
        false,
    );
    assert_eq!(s["on"], false);
    let s = ssh::summary(&none, &none, false, true);
    assert_eq!(
        (s["installed"].clone(), s["installing"].clone()),
        (json!(false), json!(true))
    );
    assert_eq!(ssh::command("robin", "pc-robin"), "ssh robin@pc-robin");
    assert_eq!(
        ssh::pick_package(&[
            "openssh-client;1:10.2;amd64;ubuntu".into(),
            "openssh-server;1:10.2p1-2ubuntu3.8;amd64;ubuntu".into()
        ]),
        Some("openssh-server;1:10.2p1-2ubuntu3.8;amd64;ubuntu".into())
    );
    assert_eq!(ssh::pick_package(&["openssh-server".into()]), None);
    assert_eq!(ssh::FILTER_NOT_INSTALLED_ARCH, 262_152);
    for good in ["ssh.socket", "ssh.service", "gnome-remote-desktop.service"] {
        assert!(systemd::unit_ok(good));
    }
    for bad in ["", "ssh", "../x.service", "a b.service"] {
        assert!(!systemd::unit_ok(bad), "{bad}");
    }
    assert!(unit("static", "activating").running() && unit("static", "activating").enabled());
    // a fresh install that left the server listening now or at the next boot (Ubuntu's postinst)
    assert!(ssh::left_on(
        &unit("enabled", "active"),
        &unit("disabled", "inactive")
    ));
    assert!(ssh::left_on(
        &unit("enabled", "inactive"),
        &unit("disabled", "inactive")
    ));
    assert!(ssh::left_on(
        &unit("disabled", "inactive"),
        &unit("disabled", "active")
    ));
    assert!(!ssh::left_on(
        &unit("disabled", "inactive"),
        &unit("disabled", "inactive")
    ));
    assert!(
        !ssh::left_on(&unit("disabled", "inactive"), &unit("indirect", "inactive")),
        "an indirect service does not start at boot by itself"
    );
    assert!(!ssh::left_on(&none, &none));
}

#[test]
fn the_secure_shell_switch_asks_polkit_as_gnome_settings_does() {
    // CheckAuthorization((sa{sv}) subject, s action, a{ss} details, u flags, s cancellation id)
    let args = ssh::check_args(":1.42");
    assert_eq!(args.type_().as_str(), "((sa{sv})sa{ss}us)");
    assert_eq!(
        args.print(true).as_str(),
        "(('system-bus-name', {'name': <':1.42'>}), 'org.gnome.controlcenter.remote-login-helper', \
         @a{ss} {}, uint32 1, '')"
    );
    let policy = std::path::Path::new(
        "/usr/share/polkit-1/actions/org.gnome.controlcenter.remote-login-helper.policy",
    );
    if let Ok(text) = std::fs::read_to_string(policy) {
        assert!(text.contains(ssh::POLKIT_ACTION));
        assert!(
            text.contains(
                "org.freedesktop.systemd1.manage-units org.freedesktop.systemd1.manage-unit-files"
            ),
            "GNOME's action implies both systemd actions"
        );
    }
    let lang = Lang::En;
    let none = std::collections::HashMap::new();
    assert!(ssh::authorization_outcome(true, &none, lang).is_ok());
    let e = ssh::authorization_outcome(false, &none, lang).unwrap_err();
    assert_eq!(
        (e.code.as_str(), e.key.as_deref()),
        ("denied", Some("core.notAuthorized"))
    );
    let dismissed =
        std::collections::HashMap::from([("polkit.dismissed".to_owned(), "true".to_owned())]);
    let e = ssh::authorization_outcome(false, &dismissed, lang).unwrap_err();
    assert_eq!(
        (e.code.as_str(), e.key.as_deref()),
        ("denied", Some("core.authCancelled"))
    );
    assert!(ssh::no_such_action(
        "GDBus.Error:org.freedesktop.PolicyKit1.Error.Failed: Action org.gnome.controlcenter.remote-login-helper is not registered"
    ));
    assert!(ssh::no_such_action(
        "GDBus.Error:org.freedesktop.DBus.Error.ServiceUnknown: The name org.freedesktop.PolicyKit1 was not provided"
    ));
    assert!(!ssh::no_such_action(
        "GDBus.Error:org.freedesktop.PolicyKit1.Error.Failed: some other failure"
    ));
    assert!(!ssh::no_such_action(
        "GDBus.Error:org.freedesktop.PolicyKit1.Error.Cancelled: dismissed"
    ));
}

#[test]
fn the_pages_keys_pass_the_allow_rows_of_their_methods() {
    use crate::services::gsettings::write_values;
    let lang = Lang::En;
    // the page's own switches (Write rows) and the switch's method
    for (method, key, value) in [
        ("gsettings.set", "view-only", json!(false)),
        ("gsettings.set", "enable", json!(true)),
        (rdp::SET_METHOD, "enable", json!(true)),
        (
            rdp::SET_METHOD,
            "tls-cert",
            json!("/home/u/.local/share/gnome-remote-desktop/certificates/rdp-tls.crt"),
        ),
        (
            rdp::SET_METHOD,
            "tls-key",
            json!("/home/u/.local/share/gnome-remote-desktop/certificates/rdp-tls.key"),
        ),
    ] {
        let r = write_values(method, rdp::RDP, None, &[(key, value)], lang, true);
        assert!(r.is_ok(), "{method} {key}: {:?}", r.err());
    }
    // the certificate belongs to sharing.rdp.set, not to the page
    let e = write_values(
        "gsettings.set",
        rdp::RDP,
        None,
        &[("tls-cert", json!("/tmp/x"))],
        lang,
        true,
    )
    .unwrap_err();
    assert_eq!(
        (e.code.as_str(), e.key.as_deref()),
        ("denied", Some("gsettings.useMethod"))
    );
    // screen-share-mode has no control in GNOME Settings 50 (gnome-a.json: "has no UI"): read only
    let e = write_values(
        "gsettings.set",
        rdp::RDP,
        None,
        &[("screen-share-mode", json!("extend"))],
        lang,
        true,
    )
    .unwrap_err();
    assert_eq!(
        (e.code.as_str(), e.key.as_deref()),
        ("denied", Some("gsettings.readOnly"))
    );
    // gsd Sharing's own list: only sharing.media.set, only at a service's path
    let schema = "org.gnome.settings-daemon.plugins.sharing.service";
    let path = "/org/gnome/settings-daemon/plugins/sharing/rygel/";
    let list = json!(["0c7e3a52-1b2d-4f6e-8a90-5d4c3b2a1f00"]);
    assert!(
        write_values(
            "sharing.media.set",
            schema,
            Some(path),
            &[("enabled-connections", list.clone())],
            lang,
            true
        )
        .is_ok()
    );
    let e = write_values(
        "gsettings.set",
        schema,
        Some(path),
        &[("enabled-connections", list.clone())],
        lang,
        true,
    )
    .unwrap_err();
    assert_eq!(e.code, "denied");
    let e = write_values(
        "sharing.media.set",
        schema,
        Some("/org/gnome/settings-daemon/plugins/sharing/rygel//"),
        &[("enabled-connections", list)],
        lang,
        true,
    )
    .unwrap_err();
    assert_eq!(e.code, "bad_args");
}

/// A Sharing whose context has `dry_run`, with a grdctl that leaves a marker when it runs.
fn sharing(dry_run: bool, dir: &Path) -> (LangContext, Arc<Sharing>) {
    let lc = LangContext::new("en", "settings");
    let ctx = if dry_run {
        let c = &lc.ctx;
        ServiceContext::new(
            c.events.clone(),
            c.config_dir.clone(),
            c.cache_dir.clone(),
            c.ui.clone(),
            c.kind.clone(),
            true,
        )
        .unwrap()
    } else {
        lc.ctx.clone()
    };
    let paths = Paths {
        grdctl: Some(fake_grdctl(dir)),
        cert_dir: dir.join("certificates"),
    };
    (lc, Sharing::with_paths(ctx, paths).unwrap())
}

/// Every dangerous call with good arguments, as the pages send them.
fn writes() -> Vec<(&'static str, Value)> {
    vec![
        ("sharing.rdp.set", json!({ "on": true })),
        ("sharing.rdp.set", json!({ "on": false })),
        (
            "sharing.rdp.credentials.set",
            json!({ "username": "alice", "password": "Geheim-12" }),
        ),
        ("sharing.rdp.system.set", json!({ "on": true })),
        (
            "sharing.rdp.system.credentials.set",
            json!({ "username": "alice", "password": "Geheim-12" }),
        ),
        ("sharing.ssh.set", json!({ "on": true })),
        ("sharing.ssh.install", json!({})),
        ("sharing.media.set", json!({ "on": true })),
        (
            "sharing.media.folders.set",
            json!({ "folders": ["/srv/media"] }),
        ),
        (
            "sharing.media.networks.remove",
            json!({ "uuid": "9b8a7c6d-5e4f-4321-8765-0fedcba98765" }),
        ),
    ]
}

#[test]
fn a_dry_run_checks_the_arguments_and_touches_nothing() {
    let dir = Dir::new("dry");
    let (_lc, s) = sharing(true, &dir.0);
    let names: Vec<&str> = writes().iter().map(|(m, _)| *m).collect();
    for (m, dangerous) in METHODS {
        assert_eq!(
            *dangerous,
            names.contains(m),
            "{m}: every dangerous method is in writes()"
        );
    }
    for (method, args) in writes() {
        let r = s.call(method, &args).expect(method);
        assert!(r.is_ok(), "{method}: {:?}", r.err());
    }
    assert!(!dir.0.join("argv").exists(), "grdctl did not run");
    assert!(
        !dir.0.join("certificates").exists(),
        "no certificate was made"
    );
    let refused = [
        ("sharing.rdp.set", json!({ "on": "yes" }), "sharing.badArgs"),
        ("sharing.ssh.set", json!({}), "sharing.badArgs"),
        (
            "sharing.rdp.credentials.set",
            json!({ "username": "-x", "password": "p" }),
            "sharing.badCredentials",
        ),
        (
            "sharing.rdp.credentials.set",
            json!({ "username": "a", "password": "a\nb" }),
            "sharing.badCredentials",
        ),
        (
            "sharing.rdp.system.credentials.set",
            json!({ "username": "a" }),
            "sharing.badCredentials",
        ),
        (
            "sharing.media.folders.set",
            json!({ "folders": ["relative"] }),
            "sharing.badFolder",
        ),
        (
            "sharing.media.networks.remove",
            json!({ "uuid": "x" }),
            "sharing.badArgs",
        ),
    ];
    for (method, args, key) in refused {
        let e = s.call(method, &args).unwrap().unwrap_err();
        assert_eq!(
            (e.code.as_str(), e.key.as_deref()),
            ("bad_args", Some(key)),
            "{method} {args}"
        );
        assert!(!e.message.contains("p\n"), "no secret in a message");
    }
    assert!(s.call("sharing.nope", &json!({})).is_none());
    s.shutdown();
}

#[test]
fn the_reads_answer_without_the_daemons() {
    let dir = Dir::new("reads");
    let (_lc, s) = sharing(false, &dir.0);
    let rdp = s.call("sharing.rdp.get", &json!({})).unwrap().unwrap();
    if rdp["available"] == true {
        // the sandbox's keyfile GSettings: GNOME's defaults
        assert_eq!(
            (
                rdp["enabled"].clone(),
                rdp["viewOnly"].clone(),
                rdp["port"].clone()
            ),
            (json!(false), json!(true), json!(3389))
        );
        assert_eq!(
            rdp["unit"]["exists"], false,
            "no user manager on the sandbox's bus"
        );
        assert_eq!(rdp["certificate"], false);
    }
    assert!(!rdp["hostname"].as_str().unwrap_or("").is_empty());
    let ssh = s.call("sharing.ssh.get", &json!({})).unwrap().unwrap();
    assert!(ssh["command"].as_str().unwrap().starts_with("ssh "));
    let media = s.call("sharing.media.get", &json!({})).unwrap().unwrap();
    assert_eq!(media["running"], false, "no gsd Sharing here");
    assert!(media["folders"].is_array());
    let system = s
        .call("sharing.rdp.system.get", &json!({}))
        .unwrap()
        .unwrap();
    assert_eq!(system["available"], false);
    s.shutdown();
}

#[test]
fn the_sharing_texts_are_in_both_languages() {
    for key in [
        "sharing.unavailable",
        "sharing.badArgs",
        "sharing.badCredentials",
        "sharing.badFolder",
        "sharing.noRemoteDesktop",
        "sharing.noRemoteLogin",
        "sharing.noTls",
        "sharing.certificateFailed",
        "sharing.credentialsFailed",
        "sharing.noSsh",
        "sharing.noPackage",
        "sharing.installFailed",
        "sharing.installRunning",
        "sharing.noSharing",
        "sharing.saveFailed",
    ] {
        let (nl, en) = (
            crate::i18n::trf(Lang::Nl, key, NO_PARAMS),
            crate::i18n::trf(Lang::En, key, NO_PARAMS),
        );
        assert!(nl != key && en != key && nl != en, "{key}: {nl} / {en}");
    }
}

// ------------------------------------------------------------------------------------- bus (dbus-fakes)

/// The calls fakes/c/sharing.py got, in order (tests/dbus-fakes/run.sh names the file).
fn fake_calls() -> Option<Vec<Value>> {
    let file = std::env::var("WIN11_FAKE_CALLS_SHARING").ok()?;
    Some(
        std::fs::read_to_string(file)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect(),
    )
}

fn calls_since(n: usize) -> Vec<Value> {
    fake_calls()
        .unwrap_or_default()
        .into_iter()
        .skip(n)
        .collect()
}

fn count() -> usize {
    fake_calls().map(|c| c.len()).unwrap_or(0)
}

fn writes_of(calls: &[Value]) -> Vec<(String, String, String, bool)> {
    calls
        .iter()
        .filter(|c| {
            matches!(
                c["method"].as_str(),
                Some(
                    "StartUnit"
                        | "StopUnit"
                        | "EnableUnitFiles"
                        | "DisableUnitFiles"
                        | "Enable"
                        | "Disable"
                        | "SetCredentials"
                        | "GetCredentials"
                        | "ImportCertificate"
                        | "EnableService"
                        | "DisableService"
                        | "InstallPackages"
                        | "Resolve"
                        | "SetHints"
                        | "CheckAuthorization"
                )
            )
        })
        .map(|c| {
            (
                c["bus"].as_str().unwrap_or("").to_owned(),
                c["method"].as_str().unwrap_or("").to_owned(),
                c["args"].as_str().unwrap_or("").to_owned(),
                c["interactive"] == true,
            )
        })
        .collect()
}

fn w(bus: &str, method: &str, args: &str, interactive: bool) -> (String, String, String, bool) {
    (bus.into(), method.into(), args.into(), interactive)
}

/// The fake on the private buses, or None (then the test says so and passes: it only runs inside run.sh).
fn bus_sharing(dir: &Path) -> Option<(LangContext, Arc<Sharing>)> {
    if fake_calls().is_none() {
        eprintln!("skipped: run inside tests/dbus-fakes/run.sh --batch c");
        return None;
    }
    Some(sharing(false, dir))
}

#[test]
#[ignore = "batch C bus test: only tests/dbus-fakes/run.sh --batch c"]
fn bus_desktop_sharing_turns_on_and_off_as_gnome_does() {
    let dir = Dir::new("bus-rdp");
    let Some((_lc, s)) = bus_sharing(&dir.0) else {
        return;
    };
    let lang = Lang::En;
    let before = count();
    let on = s
        .call("sharing.rdp.set", &json!({ "on": true }))
        .unwrap()
        .unwrap();
    assert_eq!(on["credentials"], "stored", "{on}");
    let unit = "'gnome-remote-desktop.service'";
    assert_eq!(
        writes_of(&calls_since(before)),
        [
            w(
                "session",
                "StartUnit",
                &format!("({unit}, 'replace')"),
                false
            ),
            w(
                "session",
                "EnableUnitFiles",
                &format!("([{unit}], false, false)"),
                false
            ),
        ],
        "the user unit started and enabled, without a password dialog"
    );
    // GNOME's keys: the switch and the new certificate in GNOME's place
    let item = |k: &str| {
        crate::services::gsettings::read_item(rdp::RDP, None, k, lang).unwrap()["value"].clone()
    };
    assert_eq!(item("enable"), true);
    let cert = dir.0.join("certificates").join("rdp-tls.crt");
    assert_eq!(item("tls-cert"), json!(cert.to_string_lossy()));
    assert_eq!(
        item("tls-key"),
        json!(
            dir.0
                .join("certificates")
                .join("rdp-tls.key")
                .to_string_lossy()
        )
    );
    assert!(tls::pair_ok(
        &cert,
        &dir.0.join("certificates").join("rdp-tls.key")
    ));
    // the credentials GNOME's page fills in: the login name and a new password, on grdctl's standard input
    let user = rdp::user_name();
    assert_eq!(
        lines(&dir.0.join("argv")),
        [
            "status --show-credentials".to_owned(),
            format!("rdp set-credentials {user}")
        ]
    );
    let password = lines(&dir.0.join("stdin")).pop().unwrap();
    assert_eq!(password.len(), 16);
    assert!(
        !std::fs::read_to_string(dir.0.join("argv"))
            .unwrap()
            .contains(&password)
    );
    // the page's view: running on the daemon's negotiated port, the certificate's fingerprint, the credentials
    let get = s.call("sharing.rdp.get", &json!({})).unwrap().unwrap();
    assert_eq!(
        (
            get["enabled"].clone(),
            get["running"].clone(),
            get["port"].clone()
        ),
        (json!(true), json!(true), json!(3390))
    );
    assert_eq!(
        get["fingerprint"],
        json!(tls::file_fingerprint(&cert).unwrap())
    );
    let fp = s
        .call("sharing.rdp.fingerprint", &json!({}))
        .unwrap()
        .unwrap();
    assert_eq!(fp["fingerprint"], get["fingerprint"]);
    let creds = s
        .call("sharing.rdp.credentials.get", &json!({}))
        .unwrap()
        .unwrap();
    assert_eq!(
        (creds["username"].clone(), creds["password"].clone()),
        (json!(user), json!(password))
    );
    // on again: the same certificate and credentials
    let again = s
        .call("sharing.rdp.set", &json!({ "on": true }))
        .unwrap()
        .unwrap();
    assert_eq!(again["credentials"], "kept");
    assert_eq!(
        tls::file_fingerprint(&cert),
        get["fingerprint"].as_str().map(str::to_owned)
    );
    // off: the key false, the unit stopped and disabled
    let before = count();
    s.call("sharing.rdp.set", &json!({ "on": false }))
        .unwrap()
        .unwrap();
    assert_eq!(
        writes_of(&calls_since(before)),
        [
            w(
                "session",
                "StopUnit",
                &format!("({unit}, 'replace')"),
                false
            ),
            w(
                "session",
                "DisableUnitFiles",
                &format!("([{unit}], false)"),
                false
            ),
        ]
    );
    assert_eq!(item("enable"), false);
    s.shutdown();
}

#[test]
#[ignore = "batch C bus test: only tests/dbus-fakes/run.sh --batch c"]
fn bus_remote_login_goes_through_the_configuration_daemon() {
    let dir = Dir::new("bus-login");
    let Some((_lc, s)) = bus_sharing(&dir.0) else {
        return;
    };
    let get = s
        .call("sharing.rdp.system.get", &json!({}))
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            get["available"].clone(),
            get["enabled"].clone(),
            get["certificate"].clone()
        ),
        (json!(true), json!(false), json!(false))
    );
    let before = count();
    let creds = s
        .call("sharing.rdp.credentials.get", &json!({ "system": true }))
        .unwrap()
        .unwrap();
    assert_eq!(
        (creds["username"].clone(), creds["password"].clone()),
        (Value::Null, Value::Null)
    );
    s.call(
        "sharing.rdp.system.credentials.set",
        &json!({ "username": "remote", "password": "Geheim-123" }),
    )
    .unwrap()
    .unwrap();
    let on = s
        .call("sharing.rdp.system.set", &json!({ "on": true }))
        .unwrap()
        .unwrap();
    assert_eq!(on["imported"], true);
    let calls = calls_since(before);
    assert_eq!(
        writes_of(&calls),
        [
            w("system", "GetCredentials", "", true),
            w("system", "SetCredentials", "(redacted)", true),
            w(
                "system",
                "ImportCertificate",
                "(('rdp-tls.crt', handle 0), ('rdp-tls.key', handle 1))",
                true
            ),
            w("system", "Enable", "", true),
        ],
        "every call allows GNOME's password dialog"
    );
    let import = calls
        .iter()
        .find(|c| c["method"] == "ImportCertificate")
        .unwrap();
    assert_eq!(import["files"][1], "-----BEGIN CERTIFICATE-----");
    assert!(import["files"][3].as_str().unwrap().contains("PRIVATE KEY"));
    let get = s
        .call("sharing.rdp.system.get", &json!({}))
        .unwrap()
        .unwrap();
    assert_eq!(get["enabled"], true);
    let fp = get["fingerprint"].as_str().unwrap();
    assert!(
        fp.len() == 95
            && fp
                .chars()
                .all(|c| c == ':' || c.is_ascii_digit() || c.is_ascii_uppercase()),
        "{fp}"
    );
    let creds = s
        .call("sharing.rdp.credentials.get", &json!({ "system": true }))
        .unwrap()
        .unwrap();
    assert_eq!(
        (creds["username"].clone(), creds["password"].clone()),
        (json!("remote"), json!("Geheim-123"))
    );
    // on again: the daemon has its certificate; off: Disable
    let before = count();
    let again = s
        .call("sharing.rdp.system.set", &json!({ "on": true }))
        .unwrap()
        .unwrap();
    assert_eq!(again["imported"], false);
    s.call("sharing.rdp.system.set", &json!({ "on": false }))
        .unwrap()
        .unwrap();
    assert_eq!(
        writes_of(&calls_since(before)),
        [
            w("system", "Enable", "", true),
            w("system", "Disable", "", true)
        ]
    );
    s.shutdown();
}

/// A control file of fakes/c/sharing.py next to its calls file: `forget` drops polkit's kept authorizations (as after
/// auth_admin_keep's few minutes), `dismiss` closes the next password dialog, `no-action` makes polkit not know
/// GNOME's action once (no gnome-control-center).
fn fake_control(what: &str) {
    let file = std::env::var("WIN11_FAKE_CALLS_SHARING").expect("inside run.sh");
    std::fs::write(format!("{file}.{what}"), "").unwrap();
}

/// The password dialogs polkit would have shown for these calls (the fake marks them).
fn dialogs(calls: &[Value]) -> usize {
    calls.iter().filter(|c| c["dialog"] == true).count()
}

/// The Secure Shell calls since `n`, CheckAuthorization's arguments checked and shortened (they hold the bus name).
fn ssh_writes(n: usize) -> Vec<(String, String, String, bool)> {
    writes_of(&calls_since(n))
        .into_iter()
        .map(|(bus, method, args, interactive)| {
            if method == "CheckAuthorization" {
                assert!(
                    args.contains("'system-bus-name'")
                        && args.contains("'org.gnome.controlcenter.remote-login-helper'")
                        && args.contains("uint32 1"),
                    "GNOME's action, this process, AllowUserInteraction: {args}"
                );
                (bus, method, "remote-login-helper".to_owned(), interactive)
            } else {
                (bus, method, args, interactive)
            }
        })
        .collect()
}

#[test]
#[ignore = "batch C bus test: only tests/dbus-fakes/run.sh --batch c"]
fn bus_secure_shell_is_installed_then_its_socket_turned_on_and_both_units_off() {
    let dir = Dir::new("bus-ssh");
    let Some((_lc, s)) = bus_sharing(&dir.0) else {
        return;
    };
    let get = s.call("sharing.ssh.get", &json!({})).unwrap().unwrap();
    assert_eq!(get["installed"], false);
    let e = s
        .call("sharing.ssh.set", &json!({ "on": true }))
        .unwrap()
        .unwrap_err();
    assert_eq!(e.key.as_deref(), Some("sharing.noSsh"));
    let check = w("system", "CheckAuthorization", "remote-login-helper", false);
    let off = [
        check.clone(),
        w(
            "system",
            "DisableUnitFiles",
            "(['ssh.socket', 'ssh.service'], false)",
            true,
        ),
        w("system", "StopUnit", "('ssh.socket', 'replace')", true),
        w("system", "StopUnit", "('ssh.service', 'replace')", true),
    ];
    let before = count();
    let r = s.call("sharing.ssh.install", &json!({})).unwrap().unwrap();
    assert_eq!(
        r["package"],
        "openssh-server;1:10.2p1-2ubuntu3.8;amd64;ubuntu"
    );
    assert_eq!(
        r["on"], false,
        "installed stopped, as Windows' optional feature"
    );
    let hints = "(['interactive=true', 'locale=en_GB.UTF-8'],)";
    let mut want = vec![
        w("system", "SetHints", hints, false),
        w(
            "system",
            "Resolve",
            "(uint64 262152, ['openssh-server'])",
            true,
        ),
        w("system", "SetHints", hints, false),
        w(
            "system",
            "InstallPackages",
            "(uint64 2, ['openssh-server;1:10.2p1-2ubuntu3.8;amd64;ubuntu'])",
            true,
        ),
    ];
    want.extend(off.iter().cloned());
    assert_eq!(
        ssh_writes(before),
        want,
        "Ubuntu's postinst started and enabled ssh.socket: off again, unit files first"
    );
    assert_eq!(
        dialogs(&calls_since(before)),
        1,
        "GNOME's one dialog for the off"
    );
    let get = s.call("sharing.ssh.get", &json!({})).unwrap().unwrap();
    assert_eq!(
        (
            get["installed"].clone(),
            get["on"].clone(),
            get["enabled"].clone(),
            get["installing"].clone()
        ),
        (json!(true), json!(false), json!(false), json!(false))
    );

    // on, polkit's kept authorization gone: GNOME's action asks once, systemd's two actions follow from it
    fake_control("forget");
    let before = count();
    s.call("sharing.ssh.set", &json!({ "on": true }))
        .unwrap()
        .unwrap();
    assert_eq!(
        ssh_writes(before),
        [
            check.clone(),
            w("system", "StartUnit", "('ssh.socket', 'replace')", true),
            w(
                "system",
                "EnableUnitFiles",
                "(['ssh.socket'], false, false)",
                true
            ),
        ],
        "amendment 35: the socket"
    );
    assert_eq!(
        dialogs(&calls_since(before)),
        1,
        "one password dialog, not one per systemd action"
    );
    let get = s.call("sharing.ssh.get", &json!({})).unwrap().unwrap();
    assert_eq!(
        (get["on"].clone(), get["enabled"].clone()),
        (json!(true), json!(true))
    );
    assert_eq!(
        get["command"],
        json!(ssh::command(&rdp::user_name(), &rdp::hostname()))
    );

    // off: both unit files disabled before anything stops, one dialog
    fake_control("forget");
    let before = count();
    s.call("sharing.ssh.set", &json!({ "on": false }))
        .unwrap()
        .unwrap();
    assert_eq!(
        ssh_writes(before),
        off,
        "off disables, then stops both units"
    );
    assert_eq!(dialogs(&calls_since(before)), 1);
    let get = s.call("sharing.ssh.get", &json!({})).unwrap().unwrap();
    assert_eq!(
        (get["on"].clone(), get["enabled"].clone()),
        (json!(false), json!(false))
    );

    // the dialog cancelled: systemd is not called at all, nothing is half done
    fake_control("forget");
    fake_control("dismiss");
    let before = count();
    let e = s
        .call("sharing.ssh.set", &json!({ "on": true }))
        .unwrap()
        .unwrap_err();
    assert_eq!(
        (e.code.as_str(), e.key.as_deref()),
        ("denied", Some("core.authCancelled"))
    );
    assert_eq!(ssh_writes(before), [check.clone()]);
    let get = s.call("sharing.ssh.get", &json!({})).unwrap().unwrap();
    assert_eq!(
        (get["on"].clone(), get["enabled"].clone()),
        (json!(false), json!(false))
    );

    // without GNOME's action (no gnome-control-center): the systemd calls ask themselves, as before
    fake_control("forget");
    fake_control("no-action");
    let before = count();
    s.call("sharing.ssh.set", &json!({ "on": true }))
        .unwrap()
        .unwrap();
    assert_eq!(
        ssh_writes(before),
        [
            check,
            w("system", "StartUnit", "('ssh.socket', 'replace')", true),
            w(
                "system",
                "EnableUnitFiles",
                "(['ssh.socket'], false, false)",
                true
            ),
        ]
    );
    assert_eq!(
        dialogs(&calls_since(before)),
        2,
        "manage-units and manage-unit-files, each its own dialog"
    );
    assert_eq!(
        s.call("sharing.ssh.get", &json!({})).unwrap().unwrap()["on"],
        true
    );
    s.shutdown();
}

#[test]
#[ignore = "batch C bus test: only tests/dbus-fakes/run.sh --batch c"]
fn bus_remote_login_changed_elsewhere_is_announced() {
    use std::io::BufRead;
    let dir = Dir::new("bus-login-watch");
    let Some((lc, s)) = bus_sharing(&dir.0) else {
        return;
    };
    let stream = lc.ctx.events.subscribe();
    let (tx, rx) = std::sync::mpsc::channel::<(String, Value)>();
    std::thread::spawn(move || {
        let mut name = String::new();
        for line in std::io::BufReader::new(stream)
            .lines()
            .map_while(Result::ok)
        {
            if let Some(n) = line.strip_prefix("event: ") {
                name = n.into();
            } else if let Some(data) = line.strip_prefix("data: ")
                && let Ok(v) = serde_json::from_str::<Value>(data)
            {
                let _ = tx.send((name.clone(), v));
            }
        }
    });
    // the watcher subscribes on its own thread
    std::thread::sleep(Duration::from_millis(600));
    // another program turns Remote Login on (GNOME Settings' Remote Login page, `grdctl --system rdp enable`)
    let bus = crate::services::system::common::system().unwrap();
    bus.call_sync(
        Some(rdp::CONFIG_NAME),
        rdp::CONFIG_PATH,
        rdp::CONFIG_IFACE,
        "Enable",
        None,
        None,
        gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION,
        5000,
        gio::Cancellable::NONE,
    )
    .unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let (name, v) = rx
            .recv_timeout(end.saturating_duration_since(Instant::now()))
            .expect("sharing.changed {what: system} within 5 s");
        if name == "sharing.changed" && v["what"] == "system" {
            break;
        }
    }
    let sys = s
        .call("sharing.rdp.system.get", &json!({}))
        .unwrap()
        .unwrap();
    assert_eq!(sys["enabled"], true);
    s.shutdown();
}

#[test]
#[ignore = "batch C bus test: only tests/dbus-fakes/run.sh --batch c"]
fn bus_media_sharing_goes_through_gsd_sharing_and_rygel_conf() {
    let dir = Dir::new("bus-media");
    let Some((_lc, s)) = bus_sharing(&dir.0) else {
        return;
    };
    let (lab, office) = (
        "0c7e3a52-1b2d-4f6e-8a90-5d4c3b2a1f00",
        "9b8a7c6d-5e4f-4321-8765-0fedcba98765",
    );
    let get = s.call("sharing.media.get", &json!({})).unwrap().unwrap();
    assert_eq!(
        (
            get["running"].clone(),
            get["status"].clone(),
            get["currentNetwork"].clone()
        ),
        (json!(true), json!("available"), json!(lab))
    );
    assert_eq!(
        (get["currentNetworkName"].clone(), get["enabled"].clone()),
        (json!("Lab-5G"), json!(false))
    );
    assert_eq!(
        get["networks"],
        json!([{ "uuid": office, "name": "Kantoor", "carrier": "802-3-ethernet" }])
    );
    let before = count();
    s.call("sharing.media.set", &json!({ "on": true }))
        .unwrap()
        .unwrap();
    let get = s.call("sharing.media.get", &json!({})).unwrap().unwrap();
    assert_eq!(get["enabled"], true);
    s.call("sharing.media.networks.remove", &json!({ "uuid": office }))
        .unwrap()
        .unwrap();
    s.call("sharing.media.set", &json!({ "on": false }))
        .unwrap()
        .unwrap();
    assert_eq!(
        writes_of(&calls_since(before)),
        [
            w("session", "EnableService", "('rygel',)", false),
            w(
                "session",
                "DisableService",
                &format!("('rygel', '{office}')"),
                false
            ),
            w(
                "session",
                "DisableService",
                &format!("('rygel', '{lab}')"),
                false
            ),
        ]
    );
    assert_eq!(
        s.call("sharing.media.get", &json!({})).unwrap().unwrap()["networks"],
        json!([])
    );
    // the folders: the user's rygel.conf, GNOME's preferences
    s.call(
        "sharing.media.folders.set",
        &json!({ "folders": ["/srv/media", "/data/a;b"] }),
    )
    .unwrap()
    .unwrap();
    let text = std::fs::read_to_string(glib::user_config_dir().join("rygel.conf")).unwrap();
    assert!(
        text.contains("uris=/srv/media;/data/a\\;b;") && text.contains("upnp-enabled=true"),
        "{text}"
    );
    assert_eq!(
        s.call("sharing.media.get", &json!({})).unwrap().unwrap()["folders"],
        json!(["/srv/media", "/data/a;b"])
    );
    s.shutdown();
}
