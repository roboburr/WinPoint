//! `users.*` (batch E): the user accounts of this PC, as GNOME Settings' Users panel handles them
//! (notes/claude/settings-parity.md §5.6). Native Settings' Accounts pages call these.
//!
//! * AccountsService (users/accounts.rs): `users.self`, `users.list`, `users.setRealName`, `users.setIcon`,
//!   `users.faces`, `users.setLanguage`, `users.setFormats`, `users.setAccountType`, `users.setAutoLogin`,
//!   `users.create`, `users.delete`, `users.setPassword` (a yescrypt hash through libcrypt, users/crypt.rs, or "at
//!   the next sign-in").
//! * The own password (users/passwd.rs): `users.changePassword` answers passwd's PAM prompts on a pseudo-terminal;
//!   `users.passwordQuality` is libpwquality's score and hint, or a generated password (users/quality.rs).
//! * fprintd (users/fprint.rs): `users.fingerprint.get`, `.enroll`, `.cancel`, `.delete`.
//! * An account's `language` (and `formatsLocale`) is a generated locale id, the one machine.locales lists: Ubuntu's
//!   accounts-daemon stores `nl_NL` for the `nl_NL.UTF-8` it was given, so the raw value is resolved as Ubuntu's
//!   language2locale does (machine/locale.rs `account_language`; the raw one is `languageRaw`), and the own account
//!   without a language has the session's, as GNOME's Region page shows it. `loggedIn` is logind's answer, as
//!   libaccountsservice's: GNOME Settings does not delete a user who is still logged in, nor does `users.delete`.
//! * Events: `users.changed {}` after every write here and when AccountsService reports a change made elsewhere;
//!   `users.fingerprint.progress {stage, stages, done, result}` while an enrolment runs.
//!
//! Writes ask polkit through the daemons (§5.8, `call_interactive`); under `ctx.dry_run` they check their arguments
//! and change nothing. Passwords are arguments of secret methods only (api-methods/accounts.json, amendment 53) and
//! are never logged or put in an error.
pub(crate) mod accounts;
pub(crate) mod crypt;
pub(crate) mod fprint;
pub(crate) mod passwd;
pub(crate) mod quality;
#[cfg(test)]
pub(crate) mod tests;

use super::ServiceContext;
use crate::files::{ApiError, ApiResult};
use crate::i18n::{Lang, NO_PARAMS};
use crate::services::machine::locale::{self as locales, LocaleData};
use accounts::Daemon;
use fprint::{Enrolment, Fprint};
use glib::variant::ToVariant;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

/// The methods (name, dangerous), exactly as host/src/api-methods/accounts.json lists them (a test compares).
#[cfg_attr(not(test), allow(dead_code))] // the dispatch tests compare it with the api-methods files
pub(crate) const METHODS: &[(&str, bool)] = &[
    ("users.changePassword", true),
    ("users.create", true),
    ("users.delete", true),
    ("users.faces", false),
    ("users.fingerprint.cancel", true),
    ("users.fingerprint.delete", true),
    ("users.fingerprint.enroll", true),
    ("users.fingerprint.get", false),
    ("users.list", false),
    ("users.passwordQuality", false),
    ("users.self", false),
    ("users.setAccountType", true),
    ("users.setAutoLogin", true),
    ("users.setFormats", true),
    ("users.setIcon", true),
    ("users.setLanguage", true),
    ("users.setPassword", true),
    ("users.setRealName", true),
];

/// Where GNOME's stock pictures are (gnome-control-center-faces: the avatar chooser's gallery).
pub(crate) const FACES_DIR: &str = "/usr/share/pixmaps/faces";
/// AccountsService refuses larger icon files; GNOME Settings scales a chosen picture to this size first.
const ICON_PX: i32 = 512;

/// The daemons this service talks to: None is the system bus's own (tests give fakes on their private bus).
#[derive(Clone, Default)]
pub(crate) struct Buses {
    pub accounts: Option<Daemon>,
    pub fprint: Option<Fprint>,
    /// The passwd program (tests: a script); None is `tools::system_tool("passwd")`.
    pub passwd: Option<PathBuf>,
    /// systemd-logind (tests: a fake); None is the system bus's.
    pub login: Option<Daemon>,
    /// The generated locales and main countries an account's language is resolved against (tests: fixed ones);
    /// None reads this computer's (machine/locale.rs `locale_data`).
    pub locales: Option<LocaleData>,
}

pub(crate) struct Users {
    ctx: Arc<ServiceContext>,
    buses: Buses,
    enrolment: Arc<Mutex<Option<Enrolment>>>,
    watcher: Mutex<Option<Watcher>>,
}

/// What every user of one answer is completed with: who is logged in, and the locales to resolve languages with.
struct Lookups {
    sessions: HashSet<u64>,
    locales: LocaleData,
    session_language: String,
}

fn bad(lang: Lang) -> ApiError {
    ApiError::tr("bad_args", lang, "core.badArgs", NO_PARAMS)
}

fn str_arg<'a>(args: &'a Value, key: &str, lang: Lang) -> Result<&'a str, ApiError> {
    args[key].as_str().ok_or_else(|| bad(lang))
}

fn bool_arg(args: &Value, key: &str, lang: Lang) -> Result<bool, ApiError> {
    args[key].as_bool().ok_or_else(|| bad(lang))
}

/// `uid` of the arguments, or the own uid when it is absent (and `own` allows that).
fn uid_arg(args: &Value, own: bool, lang: Lang) -> Result<u64, ApiError> {
    match &args["uid"] {
        Value::Null if own => Ok(accounts::self_uid()),
        v => v
            .as_u64()
            .filter(|u| *u < u64::from(u32::MAX))
            .ok_or_else(|| bad(lang)),
    }
}

/// A password argument: present, at most 512 bytes, no NUL or newline (passwd reads lines).
fn password_arg<'a>(args: &'a Value, key: &str, lang: Lang) -> Result<&'a str, ApiError> {
    let p = args[key]
        .as_str()
        .ok_or_else(|| ApiError::tr("bad_args", lang, "users.passwordRequired", NO_PARAMS))?;
    if p.is_empty() {
        return Err(ApiError::tr(
            "bad_args",
            lang,
            "users.passwordRequired",
            NO_PARAMS,
        ));
    }
    if p.len() > 512 || p.contains(['\0', '\n', '\r']) {
        return Err(bad(lang));
    }
    Ok(p)
}

impl Users {
    pub(crate) fn new(ctx: Arc<ServiceContext>) -> Result<Arc<Self>, String> {
        Self::with(ctx, Buses::default())
    }

    pub(crate) fn with(ctx: Arc<ServiceContext>, buses: Buses) -> Result<Arc<Self>, String> {
        Ok(Arc::new(Self {
            ctx,
            buses,
            enrolment: Arc::new(Mutex::new(None)),
            watcher: Mutex::new(None),
        }))
    }

    fn accounts(&self, lang: Lang) -> Result<Daemon, ApiError> {
        self.buses
            .accounts
            .clone()
            .or_else(Daemon::system)
            .ok_or_else(|| accounts::unavailable(lang))
    }

    fn fprint(&self) -> Option<Fprint> {
        self.buses.fprint.clone().or_else(Fprint::system)
    }

    /// The uids with a session that counts as logged in (logind, as libaccountsservice asks it).
    fn sessions(&self) -> HashSet<u64> {
        self.buses
            .login
            .clone()
            .or_else(|| Daemon::system_named(accounts::LOGIN_NAME))
            .map(|l| accounts::logged_in(&l))
            .unwrap_or_default()
    }

    fn lookups(&self) -> Lookups {
        let locales =
            self.buses.locales.clone().unwrap_or_else(|| {
                locales::locale_data(&crate::services::machine::Buses::default())
            });
        Lookups {
            sessions: self.sessions(),
            session_language: locales::session_language(&locales),
            locales,
        }
    }

    /// Follow AccountsService and fprintd from the first call on (the watcher thread, below).
    fn watch(&self) {
        let mut slot = self.watcher.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            let accounts = self.buses.accounts.clone().or_else(Daemon::system);
            *slot = Watcher::start(
                self.ctx.clone(),
                accounts,
                self.fprint(),
                self.enrolment.clone(),
            );
        }
    }

    fn changed(&self) {
        self.ctx.emit("users.changed", json!({}));
    }

    /// The `{ok, dryRun}` answer of a write under `--dry-run` (its arguments were checked).
    fn dry(&self) -> ApiResult {
        Ok(json!({"ok": true, "dryRun": true}))
    }

    pub(crate) fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
        let lang = self.ctx.lang();
        if !METHODS.iter().any(|(m, _)| *m == method) {
            return None;
        }
        self.watch();
        Some(match method {
            "users.self" => self.user_self(lang),
            "users.list" => self.list(lang),
            "users.faces" => Ok(faces(Path::new(FACES_DIR))),
            "users.setRealName" => self.set_real_name(args, lang),
            "users.setIcon" => self.set_icon(args, lang),
            "users.setLanguage" => self.set_language(args, lang),
            "users.setFormats" => self.set_formats(args, lang),
            "users.setAccountType" => self.set_account_type(args, lang),
            "users.setAutoLogin" => self.set_auto_login(args, lang),
            "users.create" => self.create(args, lang),
            "users.delete" => self.delete(args, lang),
            "users.setPassword" => self.set_password(args, lang),
            "users.changePassword" => self.change_password(args, lang),
            "users.passwordQuality" => password_quality(args, lang),
            "users.fingerprint.get" => Ok(self
                .fprint()
                .map(|f| f.get())
                .unwrap_or_else(|| json!({"available": false, "device": null, "enrolled": []}))),
            "users.fingerprint.enroll" => self.enroll(args, lang),
            "users.fingerprint.cancel" => self.enroll_cancel(),
            "users.fingerprint.delete" => self.fingerprint_delete(lang),
            _ => return None,
        })
    }

    // ------------------------------------------------------------------------------------------ AccountsService

    fn user_at(
        &self,
        daemon: &Daemon,
        path: &str,
        lang: Lang,
        look: &Lookups,
    ) -> Result<Value, ApiError> {
        let mut user = accounts::user_json(&daemon.props(path, lang)?, path, accounts::self_uid());
        complete(&mut user, look);
        Ok(user)
    }

    fn user_self(&self, lang: Lang) -> ApiResult {
        let found = self.accounts(lang).and_then(|d| {
            let path = d.find(accounts::self_uid(), lang)?;
            self.user_at(&d, &path, lang, &self.lookups())
        });
        Ok(match found {
            Ok(mut user) => {
                user["available"] = json!(true);
                user
            }
            // no AccountsService: the password database's name, and the page shows its controls as unavailable
            Err(_) => {
                let mut user = accounts::passwd_self();
                user["available"] = json!(false);
                user
            }
        })
    }

    fn list(&self, lang: Lang) -> ApiResult {
        let daemon = self.accounts(lang)?;
        let mut users = Vec::new();
        let mut paths = daemon.cached(lang)?;
        // the own account is listed even when the cache has not got it yet
        if let Ok(own) = daemon.find(accounts::self_uid(), lang)
            && !paths.contains(&own)
        {
            paths.push(own);
        }
        let look = self.lookups();
        for path in paths {
            if let Ok(user) = self.user_at(&daemon, &path, lang, &look)
                && user["systemAccount"] != true
            {
                users.push(user);
            }
        }
        users.sort_by_key(|u| {
            (
                u["self"] != true,
                u["realName"].as_str().unwrap_or("").to_lowercase(),
            )
        });
        let admins = users
            .iter()
            .filter(|u| u["accountType"] == "administrator")
            .count();
        Ok(json!({"users": users, "self": accounts::self_uid(), "administrators": admins}))
    }

    /// The object path of `uid` (the own one without `uid`).
    fn path_of(
        &self,
        daemon: &Daemon,
        args: &Value,
        own: bool,
        lang: Lang,
    ) -> Result<(u64, String), ApiError> {
        let uid = uid_arg(args, own, lang)?;
        Ok((uid, daemon.find(uid, lang)?))
    }

    /// A User Set* call with its argument checks done: the changed user.
    fn set_user(
        &self,
        args: &Value,
        own: bool,
        method: &str,
        value: glib::Variant,
        lang: Lang,
    ) -> ApiResult {
        uid_arg(args, own, lang)?;
        if self.ctx.dry_run {
            return self.dry();
        }
        let daemon = self.accounts(lang)?;
        let (_, path) = self.path_of(&daemon, args, own, lang)?;
        daemon.user_call(&path, method, value, lang)?;
        self.changed();
        self.user_at(&daemon, &path, lang, &self.lookups())
    }

    fn set_real_name(&self, args: &Value, lang: Lang) -> ApiResult {
        let name = accounts::clean_real_name(str_arg(args, "name", lang)?)
            .ok_or_else(|| ApiError::tr("bad_args", lang, "users.badName", NO_PARAMS))?;
        self.set_user(
            args,
            true,
            "SetRealName",
            (name.as_str(),).to_variant(),
            lang,
        )
    }

    fn set_icon(&self, args: &Value, lang: Lang) -> ApiResult {
        let path = str_arg(args, "path", lang)?;
        uid_arg(args, true, lang)?;
        let picture = || ApiError::tr("bad_args", lang, "users.badPicture", NO_PARAMS);
        if !path.is_empty() && (!path.starts_with('/') || !Path::new(path).is_file()) {
            return Err(picture());
        }
        if self.ctx.dry_run {
            return self.dry();
        }
        // a stock picture is copied by AccountsService as it is; any other one is scaled first, as GNOME does
        let scaled = if path.is_empty() || Path::new(path).starts_with(FACES_DIR) {
            None
        } else {
            Some(scale_icon(Path::new(path), &self.ctx.cache_dir).ok_or_else(picture)?)
        };
        let file = scaled
            .as_ref()
            .map_or(path.to_owned(), |p| p.to_string_lossy().into_owned());
        let result = self.set_user(
            args,
            true,
            "SetIconFile",
            (file.as_str(),).to_variant(),
            lang,
        );
        if let Some(tmp) = scaled {
            let _ = std::fs::remove_file(tmp);
        }
        result
    }

    fn set_language(&self, args: &Value, lang: Lang) -> ApiResult {
        let language = str_arg(args, "language", lang)?;
        if !accounts::valid_locale(language) {
            return Err(ApiError::tr(
                "bad_args",
                lang,
                "users.badLanguage",
                NO_PARAMS,
            ));
        }
        self.set_user(args, true, "SetLanguage", (language,).to_variant(), lang)
    }

    /// Regionale notatie: AccountsService's FormatsLocale (Ubuntu's SetFormatsLocale, as GNOME Settings' Formats
    /// dialog writes it next to org.gnome.system.locale region); it applies at the next sign-in.
    fn set_formats(&self, args: &Value, lang: Lang) -> ApiResult {
        let locale = str_arg(args, "locale", lang)?;
        if !accounts::valid_locale(locale) {
            return Err(ApiError::tr(
                "bad_args",
                lang,
                "users.badLanguage",
                NO_PARAMS,
            ));
        }
        self.set_user(args, true, "SetFormatsLocale", (locale,).to_variant(), lang)
    }

    fn set_account_type(&self, args: &Value, lang: Lang) -> ApiResult {
        let kind =
            accounts::account_type_value(str_arg(args, "type", lang)?).ok_or_else(|| bad(lang))?;
        let uid = uid_arg(args, false, lang)?;
        // GNOME keeps one administrator at least: the own account can't be made a standard user when it is the last
        if kind == 0 && !self.ctx.dry_run {
            let list = self.list(lang)?;
            let users = list["users"].as_array().cloned().unwrap_or_default();
            let others = users
                .iter()
                .filter(|u| u["accountType"] == "administrator" && u["uid"] != uid)
                .count();
            let target_admin = users
                .iter()
                .any(|u| u["uid"] == uid && u["accountType"] == "administrator");
            if target_admin && others == 0 {
                return Err(ApiError::tr("bad_args", lang, "users.lastAdmin", NO_PARAMS));
            }
        }
        self.set_user(args, false, "SetAccountType", (kind,).to_variant(), lang)
    }

    fn set_auto_login(&self, args: &Value, lang: Lang) -> ApiResult {
        let on = bool_arg(args, "on", lang)?;
        self.set_user(args, true, "SetAutomaticLogin", (on,).to_variant(), lang)
    }

    fn create(&self, args: &Value, lang: Lang) -> ApiResult {
        let real = accounts::clean_real_name(str_arg(args, "realName", lang)?)
            .ok_or_else(|| ApiError::tr("bad_args", lang, "users.badName", NO_PARAMS))?;
        let user_name = str_arg(args, "userName", lang)?;
        if !accounts::valid_user_name(user_name) {
            return Err(ApiError::tr(
                "bad_args",
                lang,
                "users.badUserName",
                NO_PARAMS,
            ));
        }
        if accounts::user_name_taken(user_name) {
            return Err(ApiError::tr(
                "bad_args",
                lang,
                "users.userNameTaken",
                NO_PARAMS,
            ));
        }
        let kind = accounts::account_type_value(args["accountType"].as_str().unwrap_or("standard"))
            .ok_or_else(|| bad(lang))?;
        let password = match &args["password"] {
            Value::Null => None,
            _ => Some(password_arg(args, "password", lang)?),
        };
        let hash = match password {
            Some(p) => Some(crypt::hash(p).ok_or_else(|| {
                ApiError::tr("unsupported", lang, "users.cryptUnavailable", NO_PARAMS)
            })?),
            None => None,
        };
        if self.ctx.dry_run {
            return self.dry();
        }
        let daemon = self.accounts(lang)?;
        let path = daemon.create(user_name, &real, kind, lang)?;
        // GNOME's two choices: a password now, or the user sets one at the first sign-in
        let set = match &hash {
            Some(hash) => {
                daemon.user_call(&path, "SetPassword", (hash.as_str(), "").to_variant(), lang)
            }
            None => daemon.user_call(&path, "SetPasswordMode", (1i32,).to_variant(), lang),
        };
        self.changed();
        set?;
        self.user_at(&daemon, &path, lang, &self.lookups())
    }

    fn delete(&self, args: &Value, lang: Lang) -> ApiResult {
        let uid = uid_arg(args, false, lang)?;
        let remove = bool_arg(args, "removeFiles", lang)?;
        if uid == accounts::self_uid() {
            return Err(ApiError::tr(
                "bad_args",
                lang,
                "users.deleteSelf",
                NO_PARAMS,
            ));
        }
        // GNOME's user page offers only Cancel for a user who is still logged in (act_user_is_logged_in_anywhere):
        // AccountsService's `userdel -f` would take the account and its home from under a running session
        if self.sessions().contains(&uid) {
            return Err(ApiError::tr("denied", lang, "users.loggedIn", NO_PARAMS));
        }
        if self.ctx.dry_run {
            return self.dry();
        }
        let daemon = self.accounts(lang)?;
        daemon.find(uid, lang)?;
        daemon.delete(uid, remove, lang)?;
        self.changed();
        Ok(json!({"ok": true}))
    }

    fn set_password(&self, args: &Value, lang: Lang) -> ApiResult {
        let mode = str_arg(args, "mode", lang)?;
        let value = match mode {
            "now" => {
                let p = password_arg(args, "password", lang)?;
                let hash = crypt::hash(p).ok_or_else(|| {
                    ApiError::tr("unsupported", lang, "users.cryptUnavailable", NO_PARAMS)
                })?;
                ("SetPassword", (hash.as_str(), "").to_variant())
            }
            "atLogin" => ("SetPasswordMode", (1i32,).to_variant()),
            _ => return Err(bad(lang)),
        };
        self.set_user(args, false, value.0, value.1, lang)
    }

    // ------------------------------------------------------------------------------------------ passwd

    fn change_password(&self, args: &Value, lang: Lang) -> ApiResult {
        let current = password_arg(args, "current", lang)?;
        let new = password_arg(args, "new", lang)?;
        let program = match &self.buses.passwd {
            Some(p) => Some(p.clone()),
            None => crate::tools::system_tool("passwd"),
        };
        let program = program.filter(|p| p.is_file()).ok_or_else(|| {
            ApiError::tr("unsupported", lang, "users.passwd.unavailable", NO_PARAMS)
        })?;
        if self.ctx.dry_run {
            return self.dry();
        }
        let outcome = passwd::change(&program, current, new).map_err(|_| {
            ApiError::tr("unsupported", lang, "users.passwd.unavailable", NO_PARAMS)
        })?;
        match outcome {
            passwd::Outcome::Changed => {
                self.changed();
                Ok(json!({"ok": true}))
            }
            passwd::Outcome::WrongCurrent => Err(ApiError::tr(
                "denied",
                lang,
                "users.passwd.wrongCurrent",
                NO_PARAMS,
            )),
            passwd::Outcome::TooWeak => {
                let user = accounts::passwd_self();
                let rc =
                    quality::check(new, Some(current), user["userName"].as_str()).unwrap_or(-1);
                let hint = crate::i18n::tr(lang, quality::hint_key(rc.min(-1), false));
                Err(ApiError::tr(
                    "bad_args",
                    lang,
                    "users.passwd.tooWeak",
                    &json!({"hint": hint}),
                ))
            }
            passwd::Outcome::TooSoon => Err(ApiError::tr(
                "denied",
                lang,
                "users.passwd.tooSoon",
                NO_PARAMS,
            )),
            passwd::Outcome::Unchanged => Err(ApiError::tr(
                "failed",
                lang,
                "users.passwd.unchanged",
                NO_PARAMS,
            )),
        }
    }

    // ------------------------------------------------------------------------------------------ fprintd

    fn enroll(&self, args: &Value, lang: Lang) -> ApiResult {
        let finger = args["finger"].as_str().unwrap_or(fprint::FINGERS[0]);
        if !fprint::FINGERS.contains(&finger) {
            return Err(bad(lang));
        }
        if self.ctx.dry_run {
            return self.dry();
        }
        let f = self.fprint().ok_or_else(|| {
            ApiError::tr("unsupported", lang, "users.fingerprint.none", NO_PARAMS)
        })?;
        let info = f.get();
        let device = info["device"]["path"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| {
                ApiError::tr("unsupported", lang, "users.fingerprint.none", NO_PARAMS)
            })?;
        // one enrolment at a time: a running one ends first
        self.enroll_cancel()?;
        f.claim(&device, lang)?;
        let stages = info["device"]["stages"].as_i64().unwrap_or(5);
        *self.enrolment.lock().unwrap_or_else(|e| e.into_inner()) = Some(Enrolment {
            device: device.clone(),
            stages,
            passed: 0,
        });
        if let Err(e) = f.enroll_start(&device, finger, lang) {
            self.enrolment
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            f.release(&device);
            return Err(e);
        }
        Ok(
            json!({"ok": true, "stages": stages, "scanType": info["device"]["scanType"], "finger": finger}),
        )
    }

    fn enroll_cancel(&self) -> ApiResult {
        if self.ctx.dry_run {
            return self.dry();
        }
        let running = self
            .enrolment
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let (Some(e), Some(f)) = (running, self.fprint()) {
            f.enroll_stop(&e.device);
            f.release(&e.device);
        }
        Ok(json!({"ok": true}))
    }

    fn fingerprint_delete(&self, lang: Lang) -> ApiResult {
        if self.ctx.dry_run {
            return self.dry();
        }
        let f = self.fprint().ok_or_else(|| {
            ApiError::tr("unsupported", lang, "users.fingerprint.none", NO_PARAMS)
        })?;
        let device = f.device().ok_or_else(|| {
            ApiError::tr("unsupported", lang, "users.fingerprint.none", NO_PARAMS)
        })?;
        self.enroll_cancel()?;
        f.claim(&device, lang)?;
        let result = f.delete_all(&device, lang);
        f.release(&device);
        result?;
        self.changed();
        Ok(json!({"ok": true}))
    }

    pub(crate) fn shutdown(&self) {
        let _ = self.enroll_cancel();
        if let Some(w) = self
            .watcher
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            w.stop();
        }
    }
}

/// A user of an answer completed: `loggedIn`, and `language` / `formatsLocale` as generated locale ids (the raw
/// values stay in `languageRaw` / `formatsLocaleRaw`; the own account without a language has the session's).
fn complete(user: &mut Value, look: &Lookups) {
    let uid = user["uid"].as_u64().unwrap_or(u64::MAX);
    user["loggedIn"] = json!(user["self"] == true || look.sessions.contains(&uid));
    let fallback = if user["self"] == true {
        look.session_language.as_str()
    } else {
        ""
    };
    let raw = user["languageRaw"].as_str().unwrap_or("").to_owned();
    user["language"] = json!(locales::account_language(&raw, fallback, &look.locales));
    let formats = user["formatsLocaleRaw"].as_str().unwrap_or("").to_owned();
    user["formatsLocale"] = json!(locales::account_language(&formats, "", &look.locales));
}

/// `users.passwordQuality {password, old?, userName?}` → `{level, strength, hintKey, hint, ok}`, or with `generate:
/// true` → `{password}`.
fn password_quality(args: &Value, lang: Lang) -> ApiResult {
    if args["generate"] == true {
        return quality::generate()
            .map(|p| json!({"password": p}))
            .ok_or_else(|| {
                ApiError::tr("unsupported", lang, "users.quality.unavailable", NO_PARAMS)
            });
    }
    let password = args["password"].as_str().ok_or_else(|| bad(lang))?;
    if password.len() > 512 {
        return Err(bad(lang));
    }
    let old = args["old"].as_str();
    let user = args["userName"].as_str();
    let Some(rc) = quality::check(password, old, user) else {
        // without libpwquality only the length counts, as passwd's own minimum of 8 does
        let ok = password.chars().count() >= 8;
        return Ok(
            json!({"available": false, "level": if password.is_empty() { 0 } else if ok { 3 } else { 1 }, "strength": Value::Null, "ok": ok, "hint": ""}),
        );
    };
    let empty = password.is_empty();
    let key = quality::hint_key(rc, empty);
    Ok(json!({
        "available": true,
        "level": quality::level(rc, empty),
        "strength": (f64::from(rc.max(0)) * 0.01).clamp(0.0, 1.0),
        "ok": rc >= 0,
        "hintKey": key,
        "hint": crate::i18n::tr(lang, key),
    }))
}

/// `users.faces`: GNOME's stock pictures (the avatar chooser's gallery), sorted by name.
pub(crate) fn faces(dir: &Path) -> Value {
    let mut list: Vec<Value> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let ext = path.extension()?.to_str()?.to_ascii_lowercase();
            let ok = e.file_type().ok()?.is_file()
                && matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "svg" | "webp");
            let name = path.file_stem()?.to_string_lossy().into_owned();
            ok.then(|| json!({"path": path.to_string_lossy(), "name": name}))
        })
        .collect();
    list.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    json!({"faces": list})
}

/// A chosen picture as a square PNG of at most 512 px in this host's cache (0600), for SetIconFile; None for a file
/// that is no picture.
pub(crate) fn scale_icon(source: &Path, cache: &Path) -> Option<PathBuf> {
    let pixbuf = gdk_pixbuf::Pixbuf::from_file(source).ok()?;
    let (w, h) = (pixbuf.width(), pixbuf.height());
    if w <= 0 || h <= 0 {
        return None;
    }
    // the middle square, as GNOME's crop area starts
    let side = w.min(h);
    let square = pixbuf.new_subpixbuf((w - side) / 2, (h - side) / 2, side, side);
    let px = side.min(ICON_PX);
    let scaled = if px == side {
        square
    } else {
        square.scale_simple(px, px, gdk_pixbuf::InterpType::Bilinear)?
    };
    std::fs::create_dir_all(cache).ok()?;
    let target = cache.join(format!("account-picture-{}.png", crate::events::nonce()));
    let file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&target)
            .ok()?
    };
    drop(file);
    scaled.savev(&target, "png", &[]).ok()?;
    Some(target)
}

// ------------------------------------------------------------------------------------------------- watcher

/// One `f` for a burst of a daemon's signals: the first signal starts a `delay` wait on a thread of its own, the
/// signals until it ends belong to it. Not a GLib timeout: a watcher thread iterates a MainContext of its own, and
/// glib's `timeout_add_local*` attach to the default MainContext, which the host's main thread owns, so they panic
/// there, and a panic in a signal callback aborts the host (the GNOME harness found it: a user's Changed stopped
/// Settings' backend). users.rs, goa.rs and machine.rs's watchers use it.
pub(crate) fn debounce(
    pending: &Arc<AtomicBool>,
    delay: std::time::Duration,
    f: impl FnOnce() + Send + 'static,
) {
    if pending.swap(true, Ordering::AcqRel) {
        return;
    }
    let pending = pending.clone();
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        pending.store(false, Ordering::Release);
        f();
    });
}

/// The thread that follows AccountsService (any signal of it: UserAdded, UserRemoved, a user's Changed) and fprintd's
/// EnrollStatus; its own MainContext, as gsettings.rs's worker.
struct Watcher {
    context: glib::MainContext,
    stop: Arc<AtomicBool>,
}

impl Watcher {
    fn start(
        ctx: Arc<ServiceContext>,
        accounts: Option<Daemon>,
        fprint: Option<Fprint>,
        enrolment: Arc<Mutex<Option<Enrolment>>>,
    ) -> Option<Watcher> {
        if accounts.is_none() && fprint.is_none() {
            return None;
        }
        let context = glib::MainContext::new();
        let stop = Arc::new(AtomicBool::new(false));
        let (thread_context, thread_stop) = (context.clone(), stop.clone());
        let (ready, subscribed) = std::sync::mpsc::channel::<()>();
        std::thread::Builder::new()
            .name("win11-users".into())
            .spawn(move || {
                let _ = thread_context.with_thread_default(|| {
                    let mut subscriptions = Vec::new();
                    if let Some(d) = &accounts {
                        let pending = Arc::new(AtomicBool::new(false));
                        let ctx2 = ctx.clone();
                        let id = d.bus.signal_subscribe(
                            Some(&d.name),
                            None,
                            None,
                            None,
                            None,
                            gio::DBusSignalFlags::NONE,
                            move |_, _, _, _, _, _| {
                                // a burst (a user's Changed and its PropertiesChanged) is one event
                                let ctx3 = ctx2.clone();
                                debounce(
                                    &pending,
                                    std::time::Duration::from_millis(250),
                                    move || ctx3.emit("users.changed", json!({})),
                                );
                            },
                        );
                        subscriptions.push((d.bus.clone(), id));
                    }
                    if let Some(f) = &fprint {
                        let (ctx2, enrolment2, f2) = (ctx.clone(), enrolment.clone(), f.clone());
                        let id = f.bus.signal_subscribe(
                            Some(&f.name),
                            Some(fprint::DEVICE_IFACE),
                            Some("EnrollStatus"),
                            None,
                            None,
                            gio::DBusSignalFlags::NONE,
                            move |_, _, path, _, _, params| {
                                let result = params.child_value(0).str().unwrap_or("").to_owned();
                                let done = params.child_value(1).get::<bool>().unwrap_or(false);
                                let mut slot = enrolment2.lock().unwrap_or_else(|e| e.into_inner());
                                let Some(e) = slot.as_mut().filter(|e| e.device == path) else {
                                    return;
                                };
                                let data = e.status(&result, done);
                                if done {
                                    let device = e.device.clone();
                                    slot.take();
                                    drop(slot);
                                    let f3 = f2.clone();
                                    std::thread::spawn(move || {
                                        f3.enroll_stop(&device);
                                        f3.release(&device);
                                    });
                                    ctx2.emit("users.fingerprint.progress", data);
                                    ctx2.emit("users.changed", json!({}));
                                } else {
                                    drop(slot);
                                    ctx2.emit("users.fingerprint.progress", data);
                                }
                            },
                        );
                        subscriptions.push((f.bus.clone(), id));
                    }
                    let _ = ready.send(());
                    while !thread_stop.load(Ordering::Acquire) {
                        thread_context.iteration(true);
                    }
                    for (bus, id) in subscriptions {
                        bus.signal_unsubscribe(id);
                    }
                });
            })
            .ok()?;
        // the subscriptions exist before the first call answers (an enrolment's first signal comes right after it)
        let _ = subscribed.recv_timeout(std::time::Duration::from_secs(2));
        Some(Watcher { context, stop })
    }

    fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        self.context.wakeup();
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop();
    }
}
