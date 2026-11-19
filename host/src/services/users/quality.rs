//! Password strength as GNOME Settings shows it (pw-utils.c): libpwquality through dlopen (libpwquality.so.1) with
//! the system's /etc/security/pwquality.conf, the score as a level 0-5 and a hint per libpwquality error, and a
//! generated password ("Een wachtwoord genereren").
use libc::{c_char, c_int, c_void};
use std::ffi::{CStr, CString};
use std::sync::OnceLock;

crate::dlib::dlib! {
    /// libpwquality 1 (pwquality.h)
    pub(crate) struct LibPwQuality ["libpwquality.so.1"] {
        fn pwquality_default_settings() -> *mut c_void;
        fn pwquality_free_settings(*mut c_void) -> ();
        fn pwquality_read_config(*mut c_void, *const c_char, *mut *mut c_void) -> c_int;
        fn pwquality_check(*mut c_void, *const c_char, *const c_char, *const c_char, *mut *mut c_void) -> c_int;
        fn pwquality_generate(*mut c_void, c_int, *mut *mut c_char) -> c_int;
    }
}

fn lib() -> Option<&'static LibPwQuality> {
    static LIB: OnceLock<Option<LibPwQuality>> = OnceLock::new();
    LIB.get_or_init(|| LibPwQuality::load().ok()).as_ref()
}

/// The system's settings, freed when dropped.
struct Settings<'a> {
    lib: &'a LibPwQuality,
    raw: *mut c_void,
}

impl<'a> Settings<'a> {
    fn new(lib: &'a LibPwQuality) -> Option<Settings<'a>> {
        let raw = unsafe { (lib.pwquality_default_settings)() };
        if raw.is_null() {
            return None;
        }
        // a missing or broken pwquality.conf keeps the defaults, as pw-utils.c does
        unsafe { (lib.pwquality_read_config)(raw, std::ptr::null(), std::ptr::null_mut()) };
        Some(Settings { lib, raw })
    }
}

impl Drop for Settings<'_> {
    fn drop(&mut self) {
        unsafe { (self.lib.pwquality_free_settings)(self.raw) };
    }
}

/// pwquality_check: the score 0-100, or libpwquality's negative error; None without the library.
pub(crate) fn check(password: &str, old: Option<&str>, user: Option<&str>) -> Option<i32> {
    let lib = lib()?;
    let settings = Settings::new(lib)?;
    let password = CString::new(password).ok()?;
    let old = old
        .filter(|o| !o.is_empty())
        .and_then(|o| CString::new(o).ok());
    let user = user
        .filter(|u| !u.is_empty())
        .and_then(|u| CString::new(u).ok());
    let rc = unsafe {
        (lib.pwquality_check)(
            settings.raw,
            password.as_ptr(),
            old.as_ref().map_or(std::ptr::null(), |o| o.as_ptr()),
            user.as_ref().map_or(std::ptr::null(), |u| u.as_ptr()),
            std::ptr::null_mut(),
        )
    };
    Some(rc)
}

/// pwquality_generate with GNOME's entropy (pw-utils.c pw_generate: 0 bits, the library's default).
pub(crate) fn generate() -> Option<String> {
    let lib = lib()?;
    let settings = Settings::new(lib)?;
    let mut out: *mut c_char = std::ptr::null_mut();
    let rc = unsafe { (lib.pwquality_generate)(settings.raw, 0, &mut out) };
    if rc != 0 || out.is_null() {
        return None;
    }
    let text = unsafe { CStr::from_ptr(out) }
        .to_string_lossy()
        .into_owned();
    unsafe {
        let len = libc::strlen(out);
        std::ptr::write_bytes(out, 0, len);
        libc::free(out.cast());
    }
    Some(text)
}

/// pw-utils.c pw_strength: the level 0 (empty) to 5 (strong) of a pwquality_check result.
pub(crate) fn level(rc: i32, empty: bool) -> u8 {
    if rc < 0 {
        return if empty { 0 } else { 1 };
    }
    let strength = (f64::from(rc) * 0.01).clamp(0.0, 1.0);
    if strength < 0.50 {
        2
    } else if strength < 0.75 {
        3
    } else if strength < 0.90 {
        4
    } else {
        5
    }
}

/// The catalogue key of the hint for a pwquality_check result (pw-utils.c pw_error_hint, then the level's hint).
pub(crate) fn hint_key(rc: i32, empty: bool) -> &'static str {
    match rc {
        -21 => "users.quality.same",
        -16 => "users.quality.caseOnly",
        -9 | -17 => "users.quality.similar",
        -25 => "users.quality.userName",
        -26 => "users.quality.realName",
        -28 => "users.quality.badWords",
        -22 => "users.quality.dictionary",
        -15 => "users.quality.palindrome",
        -10 => "users.quality.digits",
        -11 => "users.quality.uppers",
        -12 => "users.quality.lowers",
        -13 => "users.quality.others",
        -18 => "users.quality.classes",
        -19 => "users.quality.consecutive",
        -27 => "users.quality.classRepeat",
        -29 => "users.quality.sequence",
        -14 => "users.quality.short",
        _ if rc < 0 && empty => "users.quality.empty",
        _ if rc < 0 => "users.quality.mix",
        _ if level(rc, empty) >= 5 => "users.quality.strong",
        _ => "users.quality.stronger",
    }
}
