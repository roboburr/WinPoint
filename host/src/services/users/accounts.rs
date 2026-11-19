//! AccountsService (`org.freedesktop.Accounts` on the system bus) for `users.*`, the calls GNOME Settings' Users
//! panel makes through libaccountsservice: FindUserById, ListCachedUsers, CreateUser, DeleteUser and the User
//! object's Set* methods. Every write goes through [`call_interactive`] (notes/claude/settings-parity.md §5.8): the
//! daemon checks polkit itself (`change-own-user-data` is yes for the own account, `user-administration` and
//! `set-login-option` ask for the administrator password).
use crate::files::ApiError;
use crate::i18n::{Lang, NO_PARAMS};
use crate::services::system::common::{self, call_interactive};
use glib::variant::ToVariant;
use serde_json::{Value, json};

pub(crate) const NAME: &str = "org.freedesktop.Accounts";
pub(crate) const PATH: &str = "/org/freedesktop/Accounts";
pub(crate) const IFACE: &str = "org.freedesktop.Accounts";
pub(crate) const USER_IFACE: &str = "org.freedesktop.Accounts.User";
const TIMEOUT_MS: i32 = 5000;
/// systemd-logind, which libaccountsservice asks who is logged in.
pub(crate) const LOGIN_NAME: &str = "org.freedesktop.login1";
pub(crate) const LOGIN_PATH: &str = "/org/freedesktop/login1";
pub(crate) const LOGIN_MANAGER: &str = "org.freedesktop.login1.Manager";
pub(crate) const LOGIN_SESSION: &str = "org.freedesktop.login1.Session";

/// The daemon: a bus connection and the name to call (the well-known name, or a test fake's unique name).
#[derive(Clone)]
pub(crate) struct Daemon {
    pub bus: gio::DBusConnection,
    pub name: String,
}

impl Daemon {
    /// AccountsService on the system bus; None when the bus or the daemon is not there.
    pub(crate) fn system() -> Option<Daemon> {
        Self::system_named(NAME)
    }

    /// Another daemon on the system bus by its well-known name (logind).
    pub(crate) fn system_named(name: &str) -> Option<Daemon> {
        let bus = common::system().ok()?;
        Some(Daemon {
            bus,
            name: name.into(),
        })
    }

    fn call(
        &self,
        path: &str,
        iface: &str,
        method: &str,
        args: Option<&glib::Variant>,
    ) -> Result<glib::Variant, String> {
        common::call(&self.bus, &self.name, path, iface, method, args, TIMEOUT_MS)
    }

    /// The object path of a uid.
    pub(crate) fn find(&self, uid: u64, lang: Lang) -> Result<String, ApiError> {
        let v = self
            .call(
                PATH,
                IFACE,
                "FindUserById",
                Some(&(uid as i64,).to_variant()),
            )
            .map_err(|e| lookup_error(&e, lang))?;
        v.child_value(0)
            .str()
            .map(str::to_owned)
            .ok_or_else(|| unavailable(lang))
    }

    /// Every cached user's object path (ListCachedUsers: the non-system users GNOME lists).
    pub(crate) fn cached(&self, lang: Lang) -> Result<Vec<String>, ApiError> {
        let v = self
            .call(PATH, IFACE, "ListCachedUsers", None)
            .map_err(|_| unavailable(lang))?;
        let list = v.child_value(0);
        Ok((0..list.n_children())
            .filter_map(|i| list.child_value(i).str().map(str::to_owned))
            .collect())
    }

    /// A user's properties (GetAll of org.freedesktop.Accounts.User) as JSON.
    pub(crate) fn props(&self, path: &str, lang: Lang) -> Result<Value, ApiError> {
        common::getall(&self.bus, &self.name, path, USER_IFACE).map_err(|e| lookup_error(&e, lang))
    }

    /// A User method that asks polkit (SetRealName, SetIconFile, ...).
    pub(crate) fn user_call(
        &self,
        path: &str,
        method: &str,
        args: glib::Variant,
        lang: Lang,
    ) -> Result<(), ApiError> {
        call_interactive(
            &self.bus,
            &self.name,
            path,
            USER_IFACE,
            method,
            Some(&args),
            lang,
        )
        .map(|_| ())
    }

    /// CreateUser(name, fullname, accountType): the new user's object path.
    pub(crate) fn create(
        &self,
        user_name: &str,
        real_name: &str,
        account_type: i32,
        lang: Lang,
    ) -> Result<String, ApiError> {
        let args = (user_name, real_name, account_type).to_variant();
        let v = call_interactive(
            &self.bus,
            &self.name,
            PATH,
            IFACE,
            "CreateUser",
            Some(&args),
            lang,
        )?;
        v.child_value(0)
            .str()
            .map(str::to_owned)
            .ok_or_else(|| unavailable(lang))
    }

    /// DeleteUser(uid, removeFiles).
    pub(crate) fn delete(&self, uid: u64, remove_files: bool, lang: Lang) -> Result<(), ApiError> {
        let args = (uid as i64, remove_files).to_variant();
        call_interactive(
            &self.bus,
            &self.name,
            PATH,
            IFACE,
            "DeleteUser",
            Some(&args),
            lang,
        )
        .map(|_| ())
    }
}

/// Whether a logind session counts as logged in, as libaccountsservice counts it (act-user-manager.c
/// reload_systemd_sessions: Class "user", State not "closing"; the greeter, a lingering user's manager and a session
/// that is going do not).
pub(crate) fn session_counts(props: &Value) -> bool {
    props["Class"] == "user" && props["State"] != "closing"
}

/// The uids with a session that counts (logind's ListSessions, each session's Class and State), as
/// act_user_is_logged_in_anywhere() sees them; none when logind is not there.
pub(crate) fn logged_in(login: &Daemon) -> std::collections::HashSet<u64> {
    let mut out = std::collections::HashSet::new();
    let Ok(v) = common::call(
        &login.bus,
        &login.name,
        LOGIN_PATH,
        LOGIN_MANAGER,
        "ListSessions",
        None,
        TIMEOUT_MS,
    ) else {
        return out;
    };
    // a(susso): id, uid, user name, seat, object path
    let list = v.child_value(0);
    for i in 0..list.n_children() {
        let session = list.child_value(i);
        let (Some(uid), Some(path)) = (
            session.child_value(1).get::<u32>(),
            session.child_value(4).str().map(str::to_owned),
        ) else {
            continue;
        };
        if out.contains(&u64::from(uid)) {
            continue;
        }
        let props =
            common::getall(&login.bus, &login.name, &path, LOGIN_SESSION).unwrap_or(Value::Null);
        if session_counts(&props) {
            out.insert(u64::from(uid));
        }
    }
    out
}

pub(crate) fn unavailable(lang: Lang) -> ApiError {
    ApiError::tr("unsupported", lang, "users.unavailable", NO_PARAMS)
}

/// A failed lookup: an unknown user (AccountsService's Error.Failed "Failed to look up user") or no daemon.
fn lookup_error(text: &str, lang: Lang) -> ApiError {
    let name = common::remote_error_name(text).unwrap_or("");
    if name.starts_with("org.freedesktop.Accounts.Error") || text.contains("look up user") {
        ApiError::tr("not_found", lang, "users.noSuchUser", NO_PARAMS)
    } else {
        unavailable(lang)
    }
}

/// AccountType: 1 is an administrator (the sudo group), 0 a standard user.
pub(crate) fn account_type_name(v: &Value) -> &'static str {
    if v.as_i64() == Some(1) {
        "administrator"
    } else {
        "standard"
    }
}

/// The wire name of an account type, as AccountsService's integer.
pub(crate) fn account_type_value(name: &str) -> Option<i32> {
    match name {
        "administrator" => Some(1),
        "standard" => Some(0),
        _ => None,
    }
}

/// PasswordMode: 0 regular, 1 set at the next login, 2 none.
pub(crate) fn password_mode_name(v: &Value) -> &'static str {
    match v.as_i64() {
        Some(1) => "setAtLogin",
        Some(2) => "none",
        _ => "regular",
    }
}

/// The API's view of a user (the properties of AccountsService's User object).
pub(crate) fn user_json(props: &Value, path: &str, self_uid: u64) -> Value {
    let uid = props["Uid"].as_u64().unwrap_or(u64::MAX);
    let text = |k: &str| props[k].as_str().unwrap_or("").to_owned();
    let icon = text("IconFile");
    // AccountsService names a file even when the user never chose a picture: only an existing one is a picture
    let icon = if !icon.is_empty() && std::path::Path::new(&icon).is_file() {
        icon
    } else {
        String::new()
    };
    json!({
        "uid": uid,
        "path": path,
        "userName": text("UserName"),
        "realName": text("RealName"),
        "iconFile": icon,
        "accountType": account_type_name(&props["AccountType"]),
        "automaticLogin": props["AutomaticLogin"] == true,
        "passwordMode": password_mode_name(&props["PasswordMode"]),
        "language": text("Language"),
        "languageRaw": text("Language"),
        "formatsLocale": text("FormatsLocale"),
        "formatsLocaleRaw": text("FormatsLocale"),
        "locked": props["Locked"] == true,
        "localAccount": props["LocalAccount"] != false,
        "systemAccount": props["SystemAccount"] == true,
        "self": uid == self_uid,
    })
}

/// The own uid (the process's real uid, as GNOME Settings' `getuid()`).
pub(crate) fn self_uid() -> u64 {
    u64::from(unsafe { libc::getuid() })
}

/// The account of the own uid from the password database, for when AccountsService is not there: name and the
/// GECOS full name only.
pub(crate) fn passwd_self() -> Value {
    let uid = unsafe { libc::getuid() };
    let mut buf = vec![0 as libc::c_char; 16384];
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut out: *mut libc::passwd = std::ptr::null_mut();
    let rc = unsafe { libc::getpwuid_r(uid, &mut pw, buf.as_mut_ptr(), buf.len(), &mut out) };
    let (name, gecos) = if rc == 0 && !out.is_null() {
        let s = |p: *const libc::c_char| {
            if p.is_null() {
                String::new()
            } else {
                unsafe { std::ffi::CStr::from_ptr(p) }
                    .to_string_lossy()
                    .into_owned()
            }
        };
        (s(pw.pw_name), s(pw.pw_gecos))
    } else {
        (String::new(), String::new())
    };
    let real = gecos.split(',').next().unwrap_or("").to_owned();
    json!({"uid": u64::from(uid), "path": "", "userName": name, "realName": real, "iconFile": "", "accountType": "standard",
        "automaticLogin": false, "passwordMode": "regular", "language": "", "languageRaw": "", "formatsLocale": "",
        "formatsLocaleRaw": "", "locked": false, "localAccount": true, "systemAccount": false, "self": true,
        "loggedIn": true})
}

/// Whether a user name is in the password database (getpwnam), as GNOME's "This user name is already in use".
pub(crate) fn user_name_taken(name: &str) -> bool {
    let Ok(c) = std::ffi::CString::new(name) else {
        return false;
    };
    let mut buf = vec![0 as libc::c_char; 16384];
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut out: *mut libc::passwd = std::ptr::null_mut();
    let rc =
        unsafe { libc::getpwnam_r(c.as_ptr(), &mut pw, buf.as_mut_ptr(), buf.len(), &mut out) };
    rc == 0 && !out.is_null()
}

/// A user name GNOME Settings accepts (user-utils.c is_valid_username): 1-32 characters, lowercase letters, digits,
/// `-` and `_`, starting with a letter or `_` (useradd's NAME_REGEX on Debian and Ubuntu).
pub(crate) fn valid_user_name(name: &str) -> bool {
    let b = name.as_bytes();
    !b.is_empty()
        && b.len() <= 32
        && (b[0].is_ascii_lowercase() || b[0] == b'_')
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-' || *c == b'_')
}

/// A full name: trimmed, 1-255 characters, no control characters and no `:` or `,` (they would break the GECOS
/// field AccountsService writes).
pub(crate) fn clean_real_name(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty()
        && name.chars().count() <= 255
        && !name.chars().any(|c| c.is_control() || c == ':' || c == ','))
    .then(|| name.to_owned())
}

/// A locale AccountsService takes for Language: "" (the system's), or `ll[_CC][.codeset][@modifier]`.
pub(crate) fn valid_locale(locale: &str) -> bool {
    if locale.is_empty() {
        return true;
    }
    if locale.len() > 64 {
        return false;
    }
    let (rest, modifier) = match locale.split_once('@') {
        Some((r, m)) => (r, Some(m)),
        None => (locale, None),
    };
    let (rest, codeset) = match rest.split_once('.') {
        Some((r, c)) => (r, Some(c)),
        None => (rest, None),
    };
    let (lang, country) = match rest.split_once('_') {
        Some((l, c)) => (l, Some(c)),
        None => (rest, None),
    };
    (2..=3).contains(&lang.len())
        && lang.bytes().all(|b| b.is_ascii_lowercase())
        && country.is_none_or(|c| c.len() == 2 && c.bytes().all(|b| b.is_ascii_uppercase()))
        && codeset.is_none_or(|c| {
            !c.is_empty() && c.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
        && modifier.is_none_or(|m| !m.is_empty() && m.bytes().all(|b| b.is_ascii_lowercase()))
}
