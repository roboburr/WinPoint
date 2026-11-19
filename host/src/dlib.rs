//! Runtime bindings to C libraries through dlopen (notes/claude/settings-parity.md §6.6): the `bindings!` pattern of
//! files/archive_ffi.rs, lifted so every Settings service can use it. The build stays `--offline --locked` (no new
//! crates), and a library that is not installed only makes its feature report "not available".
//!
//! ```ignore
//! crate::dlib::dlib! {
//!     /// libqrencode 4
//!     pub(crate) struct QrEncode ["libqrencode.so.4"] {
//!         fn QRcode_encodeString(*const c_char, c_int, c_int, c_int, c_int) -> *mut c_void;
//!         fn QRcode_free(*mut c_void) -> ();
//!     }
//! }
//! static LIB: OnceLock<Result<QrEncode, DlError>> = OnceLock::new();
//! let lib = LIB.get_or_init(QrEncode::load).as_ref()?;
//! ```
//!
//! Each soname is tried in order; the first one that loads must have every symbol. The handle stays open for the
//! life of the process (the struct keeps it), as archive_ffi.rs does.
use libc::c_void;

/// Why a library could not be loaded. It holds no text, so the caller renders its own message in the caller's
/// language (a load result is usually cached for the process).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // the batches' C libraries
pub(crate) enum DlError {
    /// None of the sonames could be opened (the first one is named).
    Missing(&'static str),
    /// The library lacks this symbol.
    Symbol(&'static str),
}

#[allow(dead_code)] // through the macro
/// dlopen the first of `sonames` (each `(name, name + "\0")`) that loads.
pub(crate) fn open(sonames: &[(&'static str, &'static str)]) -> Result<*mut c_void, DlError> {
    for (_, terminated) in sonames {
        debug_assert!(terminated.ends_with('\0'));
        let handle = unsafe {
            libc::dlopen(
                terminated.as_ptr().cast(),
                libc::RTLD_NOW | libc::RTLD_LOCAL,
            )
        };
        if !handle.is_null() {
            return Ok(handle);
        }
    }
    Err(DlError::Missing(
        sonames.first().map(|(name, _)| *name).unwrap_or(""),
    ))
}

/// Whether a library can be loaded at all (it is not kept open).
#[allow(dead_code)] // the batches' availability checks (caps of a feature)
pub(crate) fn available(soname: &'static str) -> bool {
    let terminated = format!("{soname}\0");
    let handle = unsafe {
        libc::dlopen(
            terminated.as_ptr().cast(),
            libc::RTLD_LAZY | libc::RTLD_LOCAL,
        )
    };
    if handle.is_null() {
        return false;
    }
    unsafe { libc::dlclose(handle) };
    true
}

/// A struct of C function pointers and its `load()`; see the module docs.
#[allow(unused_macros)] // the batches' C libraries (libpulse, libcups, libqrencode, ...)
macro_rules! dlib {
    ($(#[$meta:meta])* $vis:vis struct $name:ident [$($so:literal),+ $(,)?] {
        $(fn $f:ident($($arg:ty),* $(,)?) -> $ret:ty;)*
    }) => {
        $(#[$meta])*
        #[allow(dead_code, non_snake_case)]
        $vis struct $name {
            _handle: usize,
            $(pub $f: unsafe extern "C" fn($($arg),*) -> $ret,)*
        }
        #[allow(dead_code)]
        impl $name {
            /// Open the library and resolve every symbol.
            $vis fn load() -> Result<Self, $crate::dlib::DlError> {
                let handle = $crate::dlib::open(&[$(($so, concat!($so, "\0"))),+])?;
                $(
                    let address = unsafe { libc::dlsym(handle, concat!(stringify!($f), "\0").as_ptr().cast()) };
                    if address.is_null() {
                        unsafe { libc::dlclose(handle) };
                        return Err($crate::dlib::DlError::Symbol(stringify!($f)));
                    }
                    // The symbol's type comes from the library's versioned public C ABI, as the caller declared it.
                    #[allow(non_snake_case)]
                    let $f = unsafe {
                        std::mem::transmute::<*mut libc::c_void, unsafe extern "C" fn($($arg),*) -> $ret>(address)
                    };
                )*
                Ok(Self { _handle: handle as usize, $($f,)* })
            }
        }
    };
}
#[allow(unused_imports)] // the batches' C libraries (libpulse, libcups, libqrencode, ...)
pub(crate) use dlib;

#[cfg(test)]
mod tests {
    use super::*;

    dlib! {
        /// libm, present on every glibc system: the loader itself under test.
        struct LibM ["libdoes-not-exist.so.0", "libm.so.6"] {
            fn cos(f64) -> f64;
            fn sqrt(f64) -> f64;
        }
    }

    dlib! {
        struct Broken ["libm.so.6"] {
            fn cos(f64) -> f64;
            fn no_such_symbol_in_libm() -> ();
        }
    }

    dlib! {
        struct Absent ["libwin11-absent-a.so.0", "libwin11-absent-b.so.1"] {
            fn anything() -> ();
        }
    }

    #[test]
    fn the_first_soname_that_loads_must_have_every_symbol() {
        let m = LibM::load().expect("libm.so.6");
        assert_eq!(unsafe { (m.sqrt)(9.0) }, 3.0);
        assert_eq!(unsafe { (m.cos)(0.0) }, 1.0);
        assert_eq!(
            Broken::load().err(),
            Some(DlError::Symbol("no_such_symbol_in_libm"))
        );
        assert_eq!(
            Absent::load().err(),
            Some(DlError::Missing("libwin11-absent-a.so.0"))
        );
        assert!(available("libm.so.6"));
        assert!(!available("libwin11-absent-a.so.0"));
    }
}
