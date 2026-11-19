//! `machine.languages` and `machine.languages.install`: the language support of Taal en regio's "Voorkeurstalen"
//! and "Een taal toevoegen" (amendment 59). GNOME Settings starts Ubuntu's language-selector for this, a Python
//! program WinPoint may not run; its data files are read here instead and the packages installed through PackageKit
//! (polkit `packagekit.package-install`, which asks for the password):
//!
//! * /usr/share/language-selector/data/languagelist: the languages to choose from (langcode, their main locale);
//!   locale2langpack: the language packs of Chinese (zh-hans, zh-hant);
//! * pkg_depends: which packages a language needs (`tr:::language-pack-`, `tr::gvfs:language-pack-gnome-`, the
//!   dictionaries and fonts of a language, each when its condition package is installed);
//! * /var/lib/dpkg/status (read only): which of them are installed.
//!
//! A language is installed when its language pack is (English always is: it is the source language). The install
//! runs on a thread of its own: Resolve the missing names, then InstallPackages of what PackageKit knows, with
//! `machine.changed {what: "languages", code, percent}` while it runs and `{.., done: true, ok, error?}` at the end.
//! Installing a language pack generates its locales (language-pack-*-base), so the language is offered as the
//! display language afterwards.
use super::{Buses, PACKAGEKIT, Peer, dry, locale, unavailable};
use crate::files::{ApiError, ApiResult};
use crate::i18n::{Lang, NO_PARAMS};
use crate::services::ServiceContext;
use glib::variant::ToVariant;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const DATA: &str = "/usr/share/language-selector/data";
const TX_IFACE: &str = "org.freedesktop.PackageKit.Transaction";
/// PackageKit's filters (PkFilterEnum as bits): not installed, newest, the native architecture (gnome-software's
/// NEWEST | ARCH): without ARCH a multiarch system's apt backend resolves a name for every architecture.
pub(crate) const FILTER_NOT_INSTALLED_NEWEST_ARCH: u64 = (1 << 3) | (1 << 16) | (1 << 18);
/// PK_TRANSACTION_FLAG_ENUM_ONLY_TRUSTED.
const FLAG_ONLY_TRUSTED: u64 = 1 << 1;
/// PK_EXIT_ENUM_SUCCESS.
const EXIT_SUCCESS: u32 = 1;
/// PK_INFO_ENUM_AVAILABLE.
const INFO_AVAILABLE: u32 = 2;
/// PK_ERROR_ENUM_TRANSACTION_CANCELLED (pk-enum.h).
pub(crate) const ERROR_TRANSACTION_CANCELLED: u32 = 17;
/// PK_ERROR_ENUM_NOT_AUTHORIZED (pk-enum.h): polkit refused, or its password dialog was dismissed (PackageKit's
/// InstallPackages returns before polkit decides; "Failed to obtain authentication." comes as ErrorCode).
pub(crate) const ERROR_NOT_AUTHORIZED: u32 = 48;
/// How long a transaction may run before the install gives up waiting for it.
const TX_LIMIT: Duration = Duration::from_secs(30 * 60);

/// One language of languagelist: its code (the language pack's: `nl`, `zh-hans`) and main locale.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Entry {
    pub code: String,
    pub locale: String,
}

/// languagelist (`Name;supported;locale;fallbacklocale;langcode;countrycode;langlist;console`): one entry per
/// language pack, in file order; C left out. Chinese is one language with two packs: locale2langpack names them.
pub(crate) fn parse_languagelist(text: &str, langpacks: &BTreeMap<String, String>) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let f: Vec<&str> = line.split(';').collect();
        if f.len() < 6 || f[4].is_empty() || f[0] == "C" {
            continue;
        }
        // Ubuntu's Dzongkha line has its fields in another order (`Dzongkha;4;dz;BT;dz_BT;UTF-8;dz_BT;`: langcode,
        // countrycode, locale, charset); read as the others it was the language "dz_BT" with the locale "BT"
        let (locale, langcode, country) = if f[4].contains('_') && !f[3].contains('_') {
            (format!("{}.{}", f[4], f[5]), f[2], f[3])
        } else {
            (
                (if f[3].is_empty() { f[2] } else { f[3] }).to_owned(),
                f[4],
                f[5],
            )
        };
        let code = langpacks
            .get(&format!("{langcode}_{country}"))
            .cloned()
            .unwrap_or_else(|| langcode.to_owned());
        if !out.iter().any(|e| e.code == code) {
            out.push(Entry { code, locale });
        }
    }
    out
}

/// locale2langpack (`zh_CN:zh-hans`): locale → language pack code.
pub(crate) fn parse_langpacks(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect()
}

/// One rule of pkg_depends: (category, language or "", condition package or "", package).
pub(crate) type Rule = (String, String, String, String);

pub(crate) fn parse_pkg_depends(text: &str) -> Vec<Rule> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.splitn(4, ':').collect();
            (f.len() == 4 && !f[3].is_empty()).then(|| {
                (
                    f[0].to_owned(),
                    f[1].to_owned(),
                    f[2].to_owned(),
                    f[3].to_owned(),
                )
            })
        })
        .collect()
}

/// The packages a language needs by the rules: a rule without a language names a prefix the code completes
/// (`language-pack-` → `language-pack-nl`), a rule of that language its package; each only when its condition
/// package is installed (or it has none).
pub(crate) fn wanted(rules: &[Rule], code: &str, installed: &HashSet<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (_, lang, cond, pkg) in rules {
        let name = if lang.is_empty() {
            if !pkg.ends_with('-') {
                continue;
            }
            format!("{pkg}{code}")
        } else if lang == code {
            pkg.clone()
        } else {
            continue;
        };
        if (cond.is_empty() || installed.contains(cond)) && !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// The installed packages of dpkg's status file ("Status: install ok installed").
pub(crate) fn parse_dpkg_status(text: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    for stanza in text.split("\n\n") {
        let mut name = None;
        let mut ok = false;
        for line in stanza.lines() {
            if let Some(n) = line.strip_prefix("Package: ") {
                name = Some(n.trim());
            } else if let Some(s) = line.strip_prefix("Status: ") {
                ok = s.trim().ends_with(" installed");
            }
        }
        if let (Some(n), true) = (name, ok) {
            out.insert(n.to_owned());
        }
    }
    out
}

/// The installed packages, read again only when the status file changed.
pub(crate) fn installed(b: &Buses) -> Arc<HashSet<String>> {
    type Cached = (
        Option<std::path::PathBuf>,
        Option<std::time::SystemTime>,
        Arc<HashSet<String>>,
    );
    static CACHE: Mutex<Option<Cached>> = Mutex::new(None);
    let path = b.file("/var/lib/dpkg/status");
    let stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((root, at, set)) = cache.as_ref()
        && *root == b.root
        && *at == stamp
        && stamp.is_some()
    {
        return set.clone();
    }
    let set = Arc::new(parse_dpkg_status(
        &std::fs::read_to_string(&path).unwrap_or_default(),
    ));
    *cache = Some((b.root.clone(), stamp, set.clone()));
    set
}

fn read_data(b: &Buses, name: &str) -> String {
    std::fs::read_to_string(b.file(&format!("{DATA}/{name}"))).unwrap_or_default()
}

/// The language pack code of a locale: locale2langpack's (`zh_TW` → `zh-hant`), else its language (`nl`).
pub(crate) fn pack_code(b: &Buses, locale: &str) -> String {
    let base = locale.split(['.', '@']).next().unwrap_or("");
    parse_langpacks(&read_data(b, "locale2langpack"))
        .get(base)
        .cloned()
        .unwrap_or_else(|| base.split('_').next().unwrap_or("").to_owned())
}

/// The codes whose language pack is installed.
pub(crate) fn installed_packs(b: &Buses) -> HashSet<String> {
    installed(b)
        .iter()
        .filter_map(|p| p.strip_prefix("language-pack-"))
        .filter(|rest| !rest.starts_with("gnome-") && !rest.ends_with("-base"))
        .map(str::to_owned)
        .collect()
}

/// The languages with what each still needs.
fn languages(b: &Buses) -> Vec<(Entry, bool, Vec<String>)> {
    let langpacks = parse_langpacks(&read_data(b, "locale2langpack"));
    let rules = parse_pkg_depends(&read_data(b, "pkg_depends"));
    let have = installed(b);
    parse_languagelist(&read_data(b, "languagelist"), &langpacks)
        .into_iter()
        .map(|e| {
            let missing: Vec<String> = wanted(&rules, &e.code, &have)
                .into_iter()
                .filter(|p| !have.contains(p))
                .collect();
            let done = e.code == "en" || have.contains(&format!("language-pack-{}", e.code));
            (e, done, missing)
        })
        .collect()
}

/// Whether PackageKit can be asked (its D-Bus service is installed, or a test gave one).
fn can_install(b: &Buses) -> bool {
    b.packagekit.is_some()
        || b.file("/usr/share/dbus-1/system-services/org.freedesktop.PackageKit.service")
            .is_file()
}

/// `machine.languages {}` → `{languages: [{code, name, endonym, locale, installed, complete, missing}], installing,
/// canInstall}`.
pub(crate) fn list(b: &Buses, install: &Mutex<Option<String>>, lang: Lang) -> Value {
    let ui = lang.code();
    let mut out: Vec<Value> = languages(b)
        .into_iter()
        .map(|(e, done, missing)| {
            let own = e.locale.split(['.', '@']).next().unwrap_or("").to_owned();
            let name = language_label(b, &e, ui);
            let endonym = language_label(b, &e, &own);
            // complete: its language packs are there (the dictionaries and fonts pkg_depends names may not exist in
            // this archive at all: PackageKit's Resolve sorts that out when installing)
            // (English, the source language, needs none)
            let complete = done && (e.code == "en" || !missing.iter().any(|p| p.starts_with("language-pack-")));
            json!({"code": e.code, "name": name, "endonym": endonym, "locale": locale::normalize(&e.locale),
                   "installed": done, "complete": complete, "missing": missing})
        })
        .collect();
    out.sort_by_key(|l| l["name"].as_str().unwrap_or("").to_lowercase());
    json!({
        "languages": out,
        "installing": install.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        "canInstall": can_install(b),
    })
}

/// A language's name: iso-codes' name of its code; Chinese (two packs) in Windows' form with its script and country,
/// "Chinees (vereenvoudigd, China)", "Chinese (Traditional, Taiwan)".
fn language_label(b: &Buses, e: &Entry, language: &str) -> String {
    let (code, country, _) = locale::parts(&e.locale);
    let name = locale::language_name(b, &code, language).unwrap_or_else(|| e.code.clone());
    if !e.code.starts_with("zh") {
        return name;
    }
    let place = country
        .as_deref()
        .and_then(|c| locale::country_name(b, c, language));
    let script = chinese_script(&e.code, country.as_deref(), language);
    let extra: Vec<&str> = [script.as_deref(), place.as_deref()]
        .into_iter()
        .flatten()
        .collect();
    if extra.is_empty() {
        name
    } else {
        format!("{name} ({})", extra.join(", "))
    }
}

/// The script of a Chinese language pack (`zh-hans`, `zh-hant`; a plain `zh` by its country: China and Singapore
/// write simplified, Taiwan, Hong Kong and Macao traditional) as Windows names it in `language`: vereenvoudigd /
/// traditioneel, Simplified / Traditional, and 简体 / 繁體 in Chinese itself.
fn chinese_script(code: &str, country: Option<&str>, language: &str) -> Option<String> {
    let simplified = match code {
        "zh-hans" => true,
        "zh-hant" => false,
        _ => match country? {
            "CN" | "SG" => true,
            "TW" | "HK" | "MO" => false,
            _ => return None,
        },
    };
    let language = language.split(['_', '.', '@']).next().unwrap_or("");
    Some(match (language, simplified) {
        ("zh", true) => "简体".to_owned(),
        ("zh", false) => "繁體".to_owned(),
        (_, true) => crate::i18n::tr(
            if language == "nl" { Lang::Nl } else { Lang::En },
            "machine.chineseSimplified",
        ),
        (_, false) => crate::i18n::tr(
            if language == "nl" { Lang::Nl } else { Lang::En },
            "machine.chineseTraditional",
        ),
    })
}

/// `machine.languages.install {code}` → `{ok, started, packages}`; the outcome comes as `machine.changed`.
pub(crate) fn install(
    ctx: &Arc<ServiceContext>,
    b: &Buses,
    slot: &Arc<Mutex<Option<String>>>,
    code: &str,
    lang: Lang,
) -> ApiResult {
    let Some((_, _, missing)) = languages(b).into_iter().find(|(e, _, _)| e.code == code) else {
        return Err(ApiError::tr(
            "bad_args",
            lang,
            "machine.badLanguage",
            NO_PARAMS,
        ));
    };
    if missing.is_empty() {
        return Ok(json!({"ok": true, "started": false, "packages": []}));
    }
    if ctx.dry_run {
        return dry();
    }
    if !can_install(b) {
        return Err(unavailable(lang, "machine.packagesUnavailable"));
    }
    let pk = b
        .packagekit()
        .ok_or_else(|| unavailable(lang, "machine.packagesUnavailable"))?;
    {
        let mut busy = slot.lock().unwrap_or_else(|e| e.into_inner());
        if busy.is_some() {
            return Err(ApiError::tr(
                "denied",
                lang,
                "machine.installBusy",
                NO_PARAMS,
            ));
        }
        *busy = Some(code.to_owned());
    }
    let (ctx2, slot2, code2, packages) =
        (ctx.clone(), slot.clone(), code.to_owned(), missing.clone());
    let arch =
        native_arch(&std::fs::read_to_string(b.file("/var/lib/dpkg/status")).unwrap_or_default());
    let started = std::thread::Builder::new()
        .name("win11-languages".into())
        .spawn(move || {
            let outcome = run(&pk, &packages, &arch, ctx2.lang(), &|percent| {
                ctx2.emit(
                    "machine.changed",
                    json!({"what": "languages", "code": code2, "percent": percent}),
                );
            });
            slot2.lock().unwrap_or_else(|e| e.into_inner()).take();
            let lang = ctx2.lang();
            let mut done =
                json!({"what": "languages", "code": code2, "done": true, "ok": outcome.is_ok()});
            if let Err(e) = outcome {
                done["error"] = json!(e.into_message(lang));
            }
            ctx2.emit("machine.changed", done);
        });
    if started.is_err() {
        slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        return Err(unavailable(lang, "machine.packagesUnavailable"));
    }
    Ok(json!({"ok": true, "started": true, "packages": missing}))
}

/// Why an install did not happen, rendered in the language of the moment it ends.
pub(crate) enum Failure {
    Api(ApiError),
    Key(&'static str),
    PackageKit(String),
}

impl Failure {
    fn into_message(self, lang: Lang) -> String {
        match self {
            Failure::Api(e) => e.message,
            Failure::Key(k) => crate::i18n::tr(lang, k),
            Failure::PackageKit(detail) => {
                crate::i18n::trf(lang, "machine.installFailed", &json!({"error": detail}))
            }
        }
    }
}

/// What one transaction said: the package ids it reported (info, id), its error (PkErrorEnum code and PackageKit's
/// detail), its exit code.
#[derive(Default)]
pub(crate) struct Outcome {
    pub packages: Vec<(u32, String)>,
    pub error: Option<(u32, String)>,
    pub exit: Option<u32>,
}

/// The Failure of a transaction's ErrorCode: a refused or dismissed password dialog is the same `denied
/// core.notAuthorized` as every other polkit write here (call_interactive), a cancelled transaction
/// machine.installStopped, anything else machine.installFailed with PackageKit's detail.
pub(crate) fn error_failure(code: u32, detail: String, lang: Lang) -> Failure {
    match code {
        ERROR_NOT_AUTHORIZED => Failure::Api(ApiError::tr(
            "denied",
            lang,
            "core.notAuthorized",
            NO_PARAMS,
        )),
        ERROR_TRANSACTION_CANCELLED => Failure::Key("machine.installStopped"),
        _ => Failure::PackageKit(detail),
    }
}

/// The dpkg architecture of this system: dpkg's own package's Architecture in dpkg's status file (dpkg is always of
/// the native architecture), else the one this program was built for.
pub(crate) fn native_arch(dpkg_status: &str) -> String {
    for stanza in dpkg_status.split("\n\n") {
        let mut name = None;
        let mut arch = None;
        for line in stanza.lines() {
            if let Some(n) = line.strip_prefix("Package: ") {
                name = Some(n.trim());
            } else if let Some(a) = line.strip_prefix("Architecture: ") {
                arch = Some(a.trim());
            }
        }
        if name == Some("dpkg")
            && let Some(a) = arch.filter(|a| !a.is_empty() && *a != "all")
        {
            return a.to_owned();
        }
    }
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "x86" => "i386",
        "aarch64" => "arm64",
        "arm" => "armhf",
        "powerpc64" => "ppc64el",
        "riscv64" => "riscv64",
        "s390x" => "s390x",
        other => other,
    }
    .to_owned()
}

/// Whether a PackageKit package id (`name;version;arch;data`) is of the native architecture or "all".
pub(crate) fn native_package(id: &str, arch: &str) -> bool {
    match id.split(';').nth(2) {
        Some(a) => a.is_empty() || a == "all" || a == arch,
        None => true,
    }
}

/// One PackageKit transaction on this thread: CreateTransaction, SetHints, the call (interactive for the install),
/// then its signals until Finished (or the daemon goes, or TX_LIMIT).
fn transaction(
    pk: &Peer,
    method: &str,
    args: glib::Variant,
    interactive: bool,
    lang: Lang,
    percent: &dyn Fn(u32),
) -> Result<Outcome, Failure> {
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let (_, root, iface) = PACKAGEKIT;
            let path = pk
                .call(root, iface, "CreateTransaction", None)
                .ok()
                .and_then(|v| v.child_value(0).str().map(str::to_owned))
                .ok_or(Failure::Key("machine.packagesUnavailable"))?;
            let seen: Arc<Mutex<Outcome>> = Arc::new(Mutex::new(Outcome::default()));
            let last_percent = Arc::new(Mutex::new(None::<u32>));
            let (seen2, last2) = (seen.clone(), last_percent.clone());
            let id = pk.bus.signal_subscribe(
                Some(&pk.name),
                None,
                None,
                Some(&path),
                None,
                gio::DBusSignalFlags::NONE,
                move |_, _, _, iface, member, params| {
                    let mut s = seen2.lock().unwrap_or_else(|e| e.into_inner());
                    match (iface, member) {
                        (TX_IFACE, "Package") => {
                            let info = params.child_value(0).get::<u32>().unwrap_or(0);
                            if let Some(id) = params.child_value(1).str() {
                                s.packages.push((info, id.to_owned()));
                            }
                        }
                        (TX_IFACE, "ErrorCode") => {
                            let code = params.child_value(0).get::<u32>().unwrap_or(0);
                            s.error = params
                                .child_value(1)
                                .str()
                                .map(|detail| (code, detail.to_owned()));
                        }
                        (TX_IFACE, "Finished") => {
                            s.exit = Some(params.child_value(0).get::<u32>().unwrap_or(0));
                        }
                        ("org.freedesktop.DBus.Properties", "PropertiesChanged") => {
                            let changed = glib::VariantDict::new(Some(&params.child_value(1)));
                            if let Some(p) = changed
                                .lookup_value("Percentage", None)
                                .and_then(|v| v.get::<u32>())
                                .filter(|p| *p <= 100)
                            {
                                *last2.lock().unwrap_or_else(|e| e.into_inner()) = Some(p);
                            }
                        }
                        _ => {}
                    }
                },
            );
            let hints = vec![
                format!("interactive={interactive}"),
                "background=false".to_owned(),
            ];
            let _ = pk.call(&path, TX_IFACE, "SetHints", Some(&(hints,).to_variant()));
            let sent = if interactive {
                pk.interactive(&path, TX_IFACE, method, &args, lang)
                    .map(|_| ())
                    .map_err(Failure::Api)
            } else {
                pk.call(&path, TX_IFACE, method, Some(&args))
                    .map(|_| ())
                    .map_err(Failure::PackageKit)
            };
            let (start, mut checked) = (Instant::now(), Instant::now());
            let mut reported = None;
            let mut gone = false;
            while sent.is_ok()
                && seen
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .exit
                    .is_none()
                && start.elapsed() < TX_LIMIT
                && !gone
            {
                if !context.iteration(false) {
                    std::thread::sleep(Duration::from_millis(20));
                }
                let p = *last_percent.lock().unwrap_or_else(|e| e.into_inner());
                if p.is_some() && p != reported {
                    reported = p;
                    percent(p.unwrap_or(0));
                }
                // the daemon went away (a crash, a restart): no Finished will come
                if checked.elapsed() > Duration::from_secs(5) {
                    checked = Instant::now();
                    gone = crate::services::system::common::call(
                        &pk.bus,
                        "org.freedesktop.DBus",
                        "/org/freedesktop/DBus",
                        "org.freedesktop.DBus",
                        "NameHasOwner",
                        Some(&(pk.name.as_str(),).to_variant()),
                        2000,
                    )
                    .ok()
                    .and_then(|v| v.child_value(0).get::<bool>())
                        == Some(false);
                }
            }
            pk.bus.signal_unsubscribe(id);
            sent?;
            let out = std::mem::take(&mut *seen.lock().unwrap_or_else(|e| e.into_inner()));
            Ok(out)
        })
        .unwrap_or(Err(Failure::Key("machine.packagesUnavailable")))
}

/// Resolve the names, then install what PackageKit has of them for this system's architecture (`arch`, dpkg's).
fn run(
    pk: &Peer,
    names: &[String],
    arch: &str,
    lang: Lang,
    percent: &dyn Fn(u32),
) -> Result<(), Failure> {
    let resolved = transaction(
        pk,
        "Resolve",
        (FILTER_NOT_INSTALLED_NEWEST_ARCH, names.to_vec()).to_variant(),
        false,
        lang,
        &|_| {},
    )?;
    let ids: Vec<String> = resolved
        .packages
        .iter()
        .filter(|(info, id)| *info == INFO_AVAILABLE && native_package(id, arch))
        .map(|(_, id)| id.clone())
        .collect();
    if ids.is_empty() {
        return match resolved.error {
            Some((code, detail)) => Err(error_failure(code, detail, lang)),
            None => Err(Failure::Key("machine.nothingToInstall")),
        };
    }
    let done = transaction(
        pk,
        "InstallPackages",
        (FLAG_ONLY_TRUSTED, ids).to_variant(),
        true,
        lang,
        percent,
    )?;
    match done.exit {
        Some(EXIT_SUCCESS) => Ok(()),
        _ => Err(done
            .error
            .map(|(code, detail)| error_failure(code, detail, lang))
            .unwrap_or(Failure::Key("machine.installStopped"))),
    }
}

#[cfg(test)]
mod chinese_tests {
    use super::*;

    #[test]
    fn chinese_is_named_with_its_script_as_windows_does() {
        assert_eq!(
            chinese_script("zh-hans", Some("CN"), "nl").as_deref(),
            Some("vereenvoudigd")
        );
        assert_eq!(
            chinese_script("zh-hant", Some("TW"), "nl").as_deref(),
            Some("traditioneel")
        );
        assert_eq!(
            chinese_script("zh-hans", Some("CN"), "en").as_deref(),
            Some("Simplified")
        );
        assert_eq!(
            chinese_script("zh-hant", Some("HK"), "en").as_deref(),
            Some("Traditional")
        );
        assert_eq!(
            chinese_script("zh-hans", Some("CN"), "zh_CN").as_deref(),
            Some("简体")
        );
        assert_eq!(
            chinese_script("zh-hant", Some("TW"), "zh_TW").as_deref(),
            Some("繁體")
        );
        // a plain zh by its country
        assert_eq!(
            chinese_script("zh", Some("SG"), "en").as_deref(),
            Some("Simplified")
        );
        assert_eq!(
            chinese_script("zh", Some("HK"), "nl").as_deref(),
            Some("traditioneel")
        );
        assert_eq!(chinese_script("zh", None, "nl").as_deref(), None);
        // the whole label, when iso-codes is installed: "Chinees (vereenvoudigd, China)"
        if std::path::Path::new("/usr/share/iso-codes/json/iso_639-2.json").is_file() {
            let b = Buses::default();
            let hans = Entry {
                code: "zh-hans".into(),
                locale: "zh_CN.UTF-8".into(),
            };
            let hant = Entry {
                code: "zh-hant".into(),
                locale: "zh_TW.UTF-8".into(),
            };
            assert_eq!(
                language_label(&b, &hans, "en"),
                "Chinese (Simplified, China)"
            );
            assert!(
                language_label(&b, &hant, "en").starts_with("Chinese (Traditional, "),
                "{}",
                language_label(&b, &hant, "en")
            );
            if std::path::Path::new("/usr/share/locale/nl/LC_MESSAGES/iso_639-2.mo").is_file() {
                assert_eq!(
                    language_label(&b, &hans, "nl"),
                    "Chinees (vereenvoudigd, China)"
                );
            }
            let dutch = Entry {
                code: "nl".into(),
                locale: "nl_NL.UTF-8".into(),
            };
            assert_eq!(
                language_label(&b, &dutch, "en"),
                "Dutch",
                "another language keeps its plain name"
            );
        }
    }
}
