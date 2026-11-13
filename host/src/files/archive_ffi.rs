//! Narrow runtime bindings to libarchive 3's public C ABI. No subprocesses are used.
//! ABI reference: libarchive v3.8.5 archive.h and archive_entry.h (upstream project).
use crate::i18n::Lang;
use libc::{c_char, c_int, c_long, c_void, size_t, ssize_t, time_t};
use std::{ffi::CStr, sync::OnceLock};

pub(super) type Handle = *mut c_void;
pub(super) type OpenCallback = Option<unsafe extern "C" fn(Handle, *mut c_void) -> c_int>;
pub(super) type ReadCallback =
    Option<unsafe extern "C" fn(Handle, *mut c_void, *mut *const c_void) -> ssize_t>;
pub(super) type SkipCallback = Option<unsafe extern "C" fn(Handle, *mut c_void, i64) -> i64>;
pub(super) type CloseCallback = Option<unsafe extern "C" fn(Handle, *mut c_void) -> c_int>;
pub(super) type SeekCallback = Option<unsafe extern "C" fn(Handle, *mut c_void, i64, c_int) -> i64>;

macro_rules! bindings {
    ($($name:ident($($argument:ty),*)->$result:ty;)*)=>{
        #[allow(dead_code)]
        pub(super) struct Library {_handle:usize,$(pub $name:unsafe extern "C" fn($($argument),*)->$result,)*}
        impl Library {
            fn load()->Result<Self,LoadError>{
                let handle=unsafe{libc::dlopen(c"libarchive.so.13".as_ptr(),libc::RTLD_NOW|libc::RTLD_LOCAL)};
                if handle.is_null(){return Err(LoadError::Missing);}
                $(let address=unsafe{libc::dlsym(handle,concat!(stringify!($name),"\0").as_ptr().cast())};
                if address.is_null(){unsafe{libc::dlclose(handle)};return Err(LoadError::Symbol(stringify!($name)));}
                // Each symbol's type below comes from the versioned public libarchive ABI.
                let $name=unsafe{std::mem::transmute::<*mut c_void,unsafe extern "C" fn($($argument),*)->$result>(address)};)*
                Ok(Self{_handle:handle as usize,$($name,)*})
            }
        }
    }
}

bindings! {
    archive_read_new()->Handle;
    archive_read_support_filter_all(Handle)->c_int;
    archive_read_support_format_zip(Handle)->c_int;
    archive_read_support_format_tar(Handle)->c_int;
    archive_read_set_seek_callback(Handle,SeekCallback)->c_int;
    archive_read_open2(Handle,*mut c_void,OpenCallback,ReadCallback,SkipCallback,CloseCallback)->c_int;
    archive_read_next_header(Handle,*mut Handle)->c_int;
    archive_read_data(Handle,*mut c_void,size_t)->ssize_t;
    archive_read_data_skip(Handle)->c_int;
    archive_read_free(Handle)->c_int;
    archive_read_header_position(Handle)->i64;
    archive_format(Handle)->c_int;
    archive_filter_count(Handle)->c_int;
    archive_filter_code(Handle,c_int)->c_int;
    archive_error_string(Handle)->*const c_char;
    archive_errno(Handle)->c_int;
    archive_write_new()->Handle;
    archive_write_set_format_zip(Handle)->c_int;
    archive_write_set_format_pax_restricted(Handle)->c_int;
    archive_write_add_filter_gzip(Handle)->c_int;
    archive_write_add_filter_bzip2(Handle)->c_int;
    archive_write_add_filter_xz(Handle)->c_int;
    archive_write_add_filter_zstd(Handle)->c_int;
    archive_write_set_options(Handle,*const c_char)->c_int;
    archive_write_set_bytes_per_block(Handle,c_int)->c_int;
    archive_write_set_bytes_in_last_block(Handle,c_int)->c_int;
    archive_write_open_fd(Handle,c_int)->c_int;
    archive_write_header(Handle,Handle)->c_int;
    archive_write_data(Handle,*const c_void,size_t)->ssize_t;
    archive_write_close(Handle)->c_int;
    archive_write_free(Handle)->c_int;
    archive_entry_new()->Handle;
    archive_entry_free(Handle)->();
    archive_entry_set_pathname_utf8(Handle,*const c_char)->();
    archive_entry_set_mode(Handle,u32)->();
    archive_entry_set_size(Handle,i64)->();
    archive_entry_set_mtime(Handle,time_t,c_long)->();
    archive_entry_set_symlink_utf8(Handle,*const c_char)->();
    archive_entry_set_hardlink_utf8(Handle,*const c_char)->();
    archive_entry_pathname_utf8(Handle)->*const c_char;
    archive_entry_pathname(Handle)->*const c_char;
    archive_entry_symlink(Handle)->*const c_char;
    archive_entry_hardlink(Handle)->*const c_char;
    archive_entry_size(Handle)->i64;
    archive_entry_filetype(Handle)->u32;
    archive_entry_perm(Handle)->u32;
    archive_entry_mtime(Handle)->time_t;
    archive_entry_mtime_nsec(Handle)->c_long;
}

/// Why libarchive could not be loaded. The load result is cached for the process, so it holds no text: the
/// message is rendered in the caller's language (round2-gate §2, "never freeze a text").
#[derive(Debug)]
pub(super) enum LoadError {
    Missing,
    Symbol(&'static str),
}
impl LoadError {
    pub(super) fn message(&self, lang: Lang) -> String {
        match self {
            LoadError::Missing => crate::i18n::tr(lang, "files.libarchiveMissing"),
            // Two pieces, as 3b898e6 built it: "De archiefbibliotheek mist " + the symbol's name.
            LoadError::Symbol(name) => crate::i18n::tr(lang, "files.libarchiveLacks") + name,
        }
    }
}

pub(super) fn library() -> Result<&'static Library, &'static LoadError> {
    static LIBRARY: OnceLock<Result<Library, LoadError>> = OnceLock::new();
    LIBRARY.get_or_init(Library::load).as_ref()
}

impl Library {
    /// libarchive's own error text, or ours in `lang` when it has none.
    pub fn error(&self, handle: Handle, lang: Lang) -> String {
        let text = unsafe { (self.archive_error_string)(handle) };
        if text.is_null() {
            crate::i18n::tr(lang, "files.archiveFailed").into()
        } else {
            unsafe { CStr::from_ptr(text) }
                .to_string_lossy()
                .into_owned()
        }
    }
}
