//! Remote Desktop: GNOME Settings' Desktop Sharing (the user's gnome-remote-desktop daemon: the
//! `org.gnome.desktop.remote-desktop.rdp` keys, the user unit `gnome-remote-desktop.service`, the credentials in the
//! login keyring through grdctl, the TLS certificate) and Remote Login (the system daemon behind
//! `org.gnome.RemoteDesktop.Configuration` on the system bus, which checks polkit
//! `org.gnome.remotedesktop.configure-system-daemon` itself and runs GNOME's enable helper on its own).
use super::systemd;
use super::tls;
use crate::files::ApiError;
use crate::i18n::{Lang, NO_PARAMS};
use crate::services::gsettings;
use crate::services::system::common::{interactive_error, variant_json};
use glib::variant::ToVariant;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub(crate) const RDP: &str = "org.gnome.desktop.remote-desktop.rdp";
pub(crate) const UNIT: &str = "gnome-remote-desktop.service";
const USER_NAME: &str = "org.gnome.RemoteDesktop.User";
const RDP_PATH: &str = "/org/gnome/RemoteDesktop/Rdp/Server";
const RDP_IFACE: &str = "org.gnome.RemoteDesktop.Rdp.Server";
pub(crate) const CONFIG_NAME: &str = "org.gnome.RemoteDesktop.Configuration";
pub(crate) const CONFIG_PATH: &str = "/org/gnome/RemoteDesktop/Configuration";
pub(crate) const CONFIG_IFACE: &str = "org.gnome.RemoteDesktop.Configuration.Rdp.Server";
/// The method that owns the certificate keys and the switch (allow/network.rs).
pub(crate) const SET_METHOD: &str = "sharing.rdp.set";
/// grdctl may wait for the keyring's unlock prompt.
const GRDCTL_SECONDS: u64 = 300;

fn failed(key: &str, error: &str, lang: Lang) -> ApiError {
    ApiError::tr(
        "failed",
        lang,
        key,
        &json!({ "error": error.chars().take(300).collect::<String>() }),
    )
}

fn unavailable(lang: Lang) -> ApiError {
    ApiError::tr("unsupported", lang, "sharing.noRemoteDesktop", NO_PARAMS)
}

/// The computer's network name (the kernel's host name, what an RDP or SSH client connects to).
pub(crate) fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_owned())
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| glib::host_name().to_string())
}

/// The user's login name (GNOME's default RDP user name).
pub(crate) fn user_name() -> String {
    glib::user_name().to_string_lossy().into_owned()
}

/// A new password as GNOME's "Generate New Password": 16 letters and digits from the kernel's random source.
pub(crate) fn random_password() -> String {
    const SET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    let mut out = String::new();
    let mut f = std::fs::File::open("/dev/urandom").ok();
    while out.len() < 16 {
        let mut b = [0u8; 32];
        match f.as_mut().map(|f| f.read_exact(&mut b)) {
            Some(Ok(())) => {}
            _ => b.iter_mut().for_each(|x| *x = glib::random_int() as u8),
        }
        // rejection sampling keeps every character equally likely
        let limit = 256 - 256 % SET.len();
        for x in b {
            if (x as usize) < limit && out.len() < 16 {
                out.push(SET[x as usize % SET.len()] as char);
            }
        }
    }
    out
}

/// A user name grdctl takes on its command line (it is no secret: Settings shows it): 1-256 bytes, no control
/// character, not an option.
pub(crate) fn username_ok(u: &str) -> bool {
    !u.is_empty() && u.len() <= 256 && !u.starts_with('-') && !u.chars().any(char::is_control)
}

/// A password grdctl reads as one line from its standard input: 1-256 bytes, no control character.
pub(crate) fn password_ok(p: &str) -> bool {
    !p.is_empty() && p.len() <= 256 && !p.chars().any(char::is_control)
}

/// The user name and password of `grdctl status --show-credentials` (LC_ALL=C): the `Username:` and `Password:`
/// lines of its `RDP:` section; "(null)", "(empty)", "(hidden)" or a missing line is no value.
pub(crate) fn parse_status(text: &str) -> (Option<String>, Option<String>) {
    let (mut section, mut user, mut pass) = (String::new(), None, None);
    let value = |v: &str| {
        let v = v.strip_prefix(' ').unwrap_or(v);
        (!v.is_empty() && !matches!(v, "(null)" | "(empty)" | "(hidden)")).then(|| v.to_owned())
    };
    for line in text.lines() {
        if !line.starts_with([' ', '\t']) {
            section = line.trim().trim_end_matches(':').to_owned();
            continue;
        }
        if section != "RDP" {
            continue;
        }
        let l = line.trim_start();
        if let Some(v) = l.strip_prefix("Username:") {
            user = value(v);
        } else if let Some(v) = l.strip_prefix("Password:") {
            pass = value(v);
        }
    }
    (user, pass)
}

/// The result of one grdctl run.
pub(crate) struct Run {
    pub ok: bool,
    pub out: String,
    pub err: String,
}

/// Run grdctl `args` (LC_ALL=C) with `input` on its standard input, at most `seconds`.
pub(crate) fn grdctl(tool: &Path, args: &[&str], input: &str, seconds: u64) -> Run {
    let spawned = Command::new(tool)
        .args(args)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let Ok(mut child) = spawned else {
        return Run {
            ok: false,
            out: String::new(),
            err: "grdctl could not be started".into(),
        };
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
    }
    let read = |r: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut data = Vec::new();
            if let Some(r) = r {
                let _ = r.take(1024 * 1024).read_to_end(&mut data);
            }
            String::from_utf8_lossy(&data).into_owned()
        })
    };
    let out = read(
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    let err = read(
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    let end = Instant::now() + Duration::from_secs(seconds);
    let ok = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Err(_) => break false,
            Ok(None) if Instant::now() >= end => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    Run {
        ok,
        out: out.join().unwrap_or_default(),
        err: err.join().unwrap_or_default(),
    }
}

/// The last line grdctl wrote about a failure (FreeRDP's log lines left out), for `data.error`.
pub(crate) fn grdctl_error(run: &Run) -> String {
    run.err
        .lines()
        .chain(run.out.lines())
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('['))
        .last()
        .unwrap_or("grdctl failed")
        .to_owned()
}

fn tool(grdctl: Option<&Path>, lang: Lang) -> Result<PathBuf, ApiError> {
    grdctl
        .filter(|p| p.is_file())
        .map(Path::to_path_buf)
        .ok_or_else(|| unavailable(lang))
}

/// The user daemon's credentials: `grdctl status --show-credentials`.
pub(crate) fn credentials(
    grdctl_path: Option<&Path>,
    lang: Lang,
) -> Result<(Option<String>, Option<String>), ApiError> {
    let tool = tool(grdctl_path, lang)?;
    let run = grdctl(&tool, &["status", "--show-credentials"], "", GRDCTL_SECONDS);
    if !run.ok {
        return Err(failed(
            "sharing.credentialsFailed",
            &grdctl_error(&run),
            lang,
        ));
    }
    Ok(parse_status(&run.out))
}

/// Store the user daemon's credentials: `grdctl rdp set-credentials <username>`, the password on its standard input
/// (never on a command line other processes can read).
pub(crate) fn set_credentials(
    grdctl_path: Option<&Path>,
    username: &str,
    password: &str,
    lang: Lang,
) -> Result<(), ApiError> {
    let tool = tool(grdctl_path, lang)?;
    let run = grdctl(
        &tool,
        &["rdp", "set-credentials", username],
        &format!("{password}\n"),
        GRDCTL_SECONDS,
    );
    if run.ok {
        Ok(())
    } else {
        Err(failed(
            "sharing.credentialsFailed",
            &grdctl_error(&run),
            lang,
        ))
    }
}

/// One key of the rdp schema (Null when the schema is not installed).
fn key(name: &str, lang: Lang) -> Value {
    gsettings::read_item(RDP, None, name, lang)
        .map(|item| {
            if item["available"] == true {
                item["value"].clone()
            } else {
                Value::Null
            }
        })
        .unwrap_or(Value::Null)
}

/// The running daemon's own RDP server (org.gnome.RemoteDesktop.User): its port, also a negotiated one.
fn live(session: Option<&gio::DBusConnection>) -> Value {
    let Some(bus) = session else {
        return Value::Null;
    };
    bus.call_sync(
        Some(USER_NAME),
        RDP_PATH,
        "org.freedesktop.DBus.Properties",
        "GetAll",
        Some(&(RDP_IFACE,).to_variant()),
        None,
        gio::DBusCallFlags::NO_AUTO_START,
        800,
        gio::Cancellable::NONE,
    )
    .map(|v| variant_json(&v.child_value(0)))
    .unwrap_or(Value::Null)
}

/// sharing.rdp.get: Desktop Sharing's state.
pub(crate) fn get(
    session: Option<&gio::DBusConnection>,
    grdctl_path: Option<&Path>,
    lang: Lang,
) -> Value {
    let enabled = key("enable", lang);
    if enabled.is_null() {
        return json!({ "available": false, "hostname": hostname() });
    }
    let unit = session.map(|b| systemd::state(b, UNIT)).unwrap_or_default();
    let live = live(session);
    let cert = key("tls-cert", lang);
    let cert_path = cert.as_str().unwrap_or("");
    let fingerprint = (!cert_path.is_empty())
        .then(|| tls::file_fingerprint(Path::new(cert_path)))
        .flatten();
    let port = live["Port"]
        .as_i64()
        .filter(|p| *p > 0)
        .map(Value::from)
        .unwrap_or_else(|| key("port", lang));
    json!({
        "available": true,
        "enabled": enabled == true,
        "viewOnly": key("view-only", lang) != false,
        "port": port,
        "configuredPort": key("port", lang),
        "negotiatePort": key("negotiate-port", lang) != false,
        "running": unit.running() || live.is_object(),
        "unit": unit.json(),
        "hostname": hostname(),
        "certificate": fingerprint.is_some(),
        "fingerprint": fingerprint,
        "grdctl": grdctl_path.is_some_and(Path::is_file),
        "tls": tls::lib().is_some(),
    })
}

/// The certificate Desktop Sharing will use: the configured pair when it loads, else a new pair in `dir`
/// (GNOME's `~/.local/share/gnome-remote-desktop/certificates/`). Some((cert, key)) when the keys must change.
pub(crate) fn ensure_certificate(
    dir: &Path,
    cert: &str,
    key: &str,
    now: i64,
    lang: Lang,
) -> Result<Option<(PathBuf, PathBuf)>, ApiError> {
    if !cert.is_empty() && !key.is_empty() && tls::pair_ok(Path::new(cert), Path::new(key)) {
        return Ok(None);
    }
    let (c, k) = (dir.join(tls::CERT_FILE), dir.join(tls::KEY_FILE));
    if tls::pair_ok(&c, &k) {
        return Ok(Some((c, k)));
    }
    let pair = tls::generate(tls::DN, now)
        .ok_or_else(|| ApiError::tr("unsupported", lang, "sharing.noTls", NO_PARAMS))?;
    tls::save(dir, &pair)
        .map(Some)
        .map_err(|e| failed("sharing.certificateFailed", &e.to_string(), lang))
}

/// What sharing.rdp.set did.
pub(crate) struct SetOutcome {
    pub credentials: &'static str,
}

/// sharing.rdp.set {on}: on = a certificate (made when needed), `enable` true, the credentials GNOME's page fills in
/// when none are stored (the user name and a new password), then the user unit started and enabled; off = `enable`
/// false, the unit stopped and disabled.
pub(crate) fn set(
    session: &gio::DBusConnection,
    on: bool,
    cert_dir: &Path,
    grdctl_path: Option<&Path>,
    lang: Lang,
) -> Result<SetOutcome, ApiError> {
    if key("enable", lang).is_null() {
        return Err(unavailable(lang));
    }
    let mut credentials = "kept";
    if on {
        let (cert, k) = (key("tls-cert", lang), key("tls-key", lang));
        let now = glib::real_time() / 1_000_000;
        if let Some((c, k)) = ensure_certificate(
            cert_dir,
            cert.as_str().unwrap_or(""),
            k.as_str().unwrap_or(""),
            now,
            lang,
        )? {
            gsettings::write_values(
                SET_METHOD,
                RDP,
                None,
                &[
                    ("tls-cert", json!(c.to_string_lossy())),
                    ("tls-key", json!(k.to_string_lossy())),
                ],
                lang,
                false,
            )?;
        }
        gsettings::write_values(
            SET_METHOD,
            RDP,
            None,
            &[("enable", json!(true))],
            lang,
            false,
        )?;
        credentials = match credentials_of(grdctl_path, lang) {
            Some((Some(_), Some(_))) => "kept",
            Some((user, _)) => {
                let user = user.unwrap_or_else(user_name);
                match set_credentials(grdctl_path, &user, &random_password(), lang) {
                    Ok(()) => "stored",
                    Err(_) => "failed",
                }
            }
            None => "failed",
        };
        systemd::enable(session, UNIT, false, lang)?;
    } else {
        gsettings::write_values(
            SET_METHOD,
            RDP,
            None,
            &[("enable", json!(false))],
            lang,
            false,
        )?;
        if systemd::state(session, UNIT).exists {
            systemd::disable(session, UNIT, false, lang)?;
        }
    }
    Ok(SetOutcome { credentials })
}

fn credentials_of(
    grdctl_path: Option<&Path>,
    lang: Lang,
) -> Option<(Option<String>, Option<String>)> {
    credentials(grdctl_path, lang).ok()
}

// ------------------------------------------------------------------------------------- the system daemon

fn config_call(
    bus: &gio::DBusConnection,
    method: &str,
    args: Option<&glib::Variant>,
    lang: Lang,
) -> Result<glib::Variant, ApiError> {
    crate::services::system::common::call_interactive(
        bus,
        CONFIG_NAME,
        CONFIG_PATH,
        CONFIG_IFACE,
        method,
        args,
        lang,
    )
}

/// sharing.rdp.system.get: Remote Login's daemon (D-Bus activated as the gnome-remote-desktop user): Enabled,
/// Port, TlsCert, TlsFingerprint; available false when the system has no such service.
pub(crate) fn system_get(bus: Option<&gio::DBusConnection>) -> Value {
    let Some(bus) = bus else {
        return json!({ "available": false });
    };
    match bus.call_sync(
        Some(CONFIG_NAME),
        CONFIG_PATH,
        "org.freedesktop.DBus.Properties",
        "GetAll",
        Some(&(CONFIG_IFACE,).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        5000,
        gio::Cancellable::NONE,
    ) {
        Ok(v) => {
            let p = variant_json(&v.child_value(0));
            let fp = p["TlsFingerprint"].as_str().unwrap_or("");
            json!({
                "available": true,
                "enabled": p["Enabled"] == true,
                "port": p["Port"],
                "certificate": !p["TlsCert"].as_str().unwrap_or("").is_empty() || !fp.is_empty(),
                "fingerprint": (!fp.is_empty()).then(|| normal_fingerprint(fp)),
                "hostname": hostname(),
            })
        }
        Err(_) => json!({ "available": false }),
    }
}

/// A daemon's fingerprint text (`ab:cd…` or `AB CD…`) in this page's form.
pub(crate) fn normal_fingerprint(fp: &str) -> String {
    let hex: String = fp.chars().filter(char::is_ascii_hexdigit).collect();
    hex.to_ascii_uppercase()
        .as_bytes()
        .chunks(2)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join(":")
}

/// GetCredentials (polkit configure-system-daemon, interactive) → (user name, password).
pub(crate) fn system_credentials(
    bus: &gio::DBusConnection,
    lang: Lang,
) -> Result<(Option<String>, Option<String>), ApiError> {
    let v = config_call(bus, "GetCredentials", None, lang)?;
    let c = variant_json(&v.child_value(0));
    let text = |k: &str| c[k].as_str().filter(|s| !s.is_empty()).map(str::to_owned);
    Ok((text("username"), text("password")))
}

/// SetCredentials({username, password}), interactive.
pub(crate) fn system_set_credentials(
    bus: &gio::DBusConnection,
    username: &str,
    password: &str,
    lang: Lang,
) -> Result<(), ApiError> {
    let creds: BTreeMap<String, glib::Variant> = [
        ("username".to_owned(), username.to_variant()),
        ("password".to_owned(), password.to_variant()),
    ]
    .into();
    config_call(
        bus,
        "SetCredentials",
        Some(&glib::Variant::tuple_from_iter([creds.to_variant()])),
        lang,
    )?;
    Ok(())
}

/// A new private (0700) directory `gnome-remote-desktop-XXXXXX` in the temporary directory, as GNOME's.
fn private_tmp_dir() -> std::io::Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    let base = glib::tmp_dir();
    let mut last = std::io::Error::other("no temporary directory");
    for _ in 0..16 {
        let name: String = (0..6)
            .map(|_| {
                let set = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
                set[glib::random_int_range(0, set.len() as i32) as usize] as char
            })
            .collect();
        let dir = base.join(format!("gnome-remote-desktop-{name}"));
        match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// ImportCertificate((sh) certificate, (sh) private key) with both files as file descriptors (GNOME's freshly made
/// pair in a private temporary directory, removed again), interactive.
pub(crate) fn system_import(
    bus: &gio::DBusConnection,
    pair: &tls::Pair,
    lang: Lang,
) -> Result<(), ApiError> {
    use gio::prelude::UnixFDListExtManual;
    use std::os::fd::AsRawFd;
    let dir =
        private_tmp_dir().map_err(|e| failed("sharing.certificateFailed", &e.to_string(), lang))?;
    let result = (|| {
        let (cert, key) = tls::save(&dir, pair)
            .map_err(|e| failed("sharing.certificateFailed", &e.to_string(), lang))?;
        let open = |p: &Path| {
            std::fs::File::open(p)
                .map_err(|e| failed("sharing.certificateFailed", &e.to_string(), lang))
        };
        let (cf, kf) = (open(&cert)?, open(&key)?);
        let fds = gio::UnixFDList::new();
        let ci = fds
            .append(cf.as_raw_fd())
            .map_err(|e| failed("sharing.certificateFailed", &e.to_string(), lang))?;
        let ki = fds
            .append(kf.as_raw_fd())
            .map_err(|e| failed("sharing.certificateFailed", &e.to_string(), lang))?;
        let args = glib::Variant::tuple_from_iter([
            (tls::CERT_FILE, glib::variant::Handle(ci)).to_variant(),
            (tls::KEY_FILE, glib::variant::Handle(ki)).to_variant(),
        ]);
        // a method call message of its own: gio-rs 0.18's call_with_unix_fd_list_sync panics on a reply without
        // descriptors (it takes the reply's NULL fd list as an owned object)
        let msg = gio::DBusMessage::new_method_call(
            Some(CONFIG_NAME),
            CONFIG_PATH,
            Some(CONFIG_IFACE),
            "ImportCertificate",
        );
        msg.set_body(&args);
        msg.set_unix_fd_list(Some(&fds));
        msg.set_flags(gio::DBusMessageFlags::ALLOW_INTERACTIVE_AUTHORIZATION);
        let (reply, _) = bus
            .send_message_with_reply_sync(
                &msg,
                gio::DBusSendMessageFlags::NONE,
                crate::services::system::common::INTERACTIVE_TIMEOUT_MS,
                gio::Cancellable::NONE,
            )
            .map_err(|e| interactive_error(&e.to_string(), lang))?;
        // the reply's type checked here: gio-rs 0.18's to_gerror asserts wrongly on a method return in debug builds
        if reply.message_type() == gio::DBusMessageType::Error {
            let name = reply
                .error_name()
                .map(|n| n.to_string())
                .unwrap_or_default();
            let text = reply
                .body()
                .and_then(|b| b.try_child_value(0))
                .and_then(|v| v.str().map(str::to_owned))
                .unwrap_or_default();
            return Err(interactive_error(
                &format!("GDBus.Error:{name}: {text}"),
                lang,
            ));
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// sharing.rdp.system.set {on}: on = GNOME's certificate imported when the daemon has none, then Enable(); off =
/// Disable(). Both interactive (the daemon checks polkit and starts gnome-remote-desktop.service through
/// gnome-remote-desktop-enable-service itself).
pub(crate) fn system_set(
    bus: &gio::DBusConnection,
    on: bool,
    lang: Lang,
) -> Result<Value, ApiError> {
    let state = system_get(Some(bus));
    if state["available"] != true {
        return Err(ApiError::tr(
            "unsupported",
            lang,
            "sharing.noRemoteLogin",
            NO_PARAMS,
        ));
    }
    let mut imported = false;
    if on {
        if state["certificate"] != true {
            let now = glib::real_time() / 1_000_000;
            let pair = tls::generate(tls::DN, now)
                .ok_or_else(|| ApiError::tr("unsupported", lang, "sharing.noTls", NO_PARAMS))?;
            system_import(bus, &pair, lang)?;
            imported = true;
        }
        config_call(bus, "Enable", None, lang)?;
    } else {
        config_call(bus, "Disable", None, lang)?;
    }
    Ok(json!({ "on": on, "imported": imported }))
}
