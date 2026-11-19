//! Password hashes for AccountsService's SetPassword, as GNOME Settings makes them (user-utils.c make_crypted):
//! libxcrypt's own default method (`crypt_gensalt_rn` with no prefix: yescrypt `$y$` on Debian and Ubuntu) and
//! `crypt_rn`, through dlopen (libcrypt.so.1), so the build needs no new crate.
use libc::{c_char, c_int, c_ulong, c_void};
use std::ffi::{CStr, CString};
use std::sync::OnceLock;

crate::dlib::dlib! {
    /// libxcrypt (crypt.h)
    pub(crate) struct LibCrypt ["libcrypt.so.1", "libcrypt.so.2"] {
        fn crypt_gensalt_rn(*const c_char, c_ulong, *const c_char, c_int, *mut c_char, c_int) -> *mut c_char;
        fn crypt_rn(*const c_char, *const c_char, *mut c_void, c_int) -> *mut c_char;
    }
}

/// sizeof(struct crypt_data) in libxcrypt 4 (crypt.h: CRYPT_OUTPUT_SIZE + CRYPT_MAX_PASSPHRASE_SIZE + ... = 32768).
const CRYPT_DATA_SIZE: usize = 32768;
/// CRYPT_GENSALT_OUTPUT_SIZE.
const SETTING_SIZE: usize = 192;

fn lib() -> Option<&'static LibCrypt> {
    static LIB: OnceLock<Option<LibCrypt>> = OnceLock::new();
    LIB.get_or_init(|| LibCrypt::load().ok()).as_ref()
}

/// Whether hashes can be made here (tests: the crypt tests skip without libcrypt).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn available() -> bool {
    lib().is_some()
}

/// crypt_rn of `phrase` with `setting` (a salt, or a whole hash to check against).
fn crypt_with(lib: &LibCrypt, phrase: &str, setting: &CStr) -> Option<String> {
    let phrase = CString::new(phrase).ok()?;
    let mut data = vec![0u8; CRYPT_DATA_SIZE];
    let out = unsafe {
        (lib.crypt_rn)(
            phrase.as_ptr(),
            setting.as_ptr(),
            data.as_mut_ptr().cast(),
            CRYPT_DATA_SIZE as c_int,
        )
    };
    let hash = (!out.is_null()).then(|| {
        unsafe { CStr::from_ptr(out) }
            .to_string_lossy()
            .into_owned()
    });
    data.iter_mut().for_each(|b| *b = 0); // the passphrase's work area
    // libxcrypt's failure tokens start with '*'
    hash.filter(|h| h.starts_with('$'))
}

/// A new hash of `phrase` with a fresh random salt of the system's default method.
pub(crate) fn hash(phrase: &str) -> Option<String> {
    let lib = lib()?;
    let mut setting = [0 as c_char; SETTING_SIZE];
    let salt = unsafe {
        (lib.crypt_gensalt_rn)(
            std::ptr::null(),
            0,
            std::ptr::null(),
            0,
            setting.as_mut_ptr(),
            SETTING_SIZE as c_int,
        )
    };
    if salt.is_null() {
        return None;
    }
    let setting = unsafe { CStr::from_ptr(setting.as_ptr()) }.to_owned();
    crypt_with(lib, phrase, &setting)
}

/// Whether `phrase` gives `hash` (tests: a hash made here verifies).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn verify(phrase: &str, hash: &str) -> bool {
    let (Some(lib), Ok(setting)) = (lib(), CString::new(hash)) else {
        return false;
    };
    crypt_with(lib, phrase, &setting).is_some_and(|h| h == hash)
}
