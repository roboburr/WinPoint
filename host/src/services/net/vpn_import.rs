//! Offline NetworkManager import. Parsing creates no NMClient and contacts no bus; the caller saves the typed
//! result with AddConnection2 and interactive authorization. No GUI editor is instantiated.
use super::settings_map::{self, Groups};
use crate::files::ApiError;
use crate::i18n::{Lang, NO_PARAMS};
use glib::translate::from_glib_full;
use libc::{c_char, c_int, c_uint, c_void};
use serde_json::json;
use std::ffi::CString;
use std::path::Path;
use std::sync::OnceLock;

crate::dlib::dlib! {
    /// libnm 1.40 or newer (WireGuard's offline importer).
    pub(crate) struct LibNm ["libnm.so.0"] {
        fn nm_vpn_plugin_info_new_from_file(*const c_char, *mut *mut glib::ffi::GError) -> *mut c_void;
        fn nm_vpn_plugin_info_load_editor_plugin(*mut c_void, *mut *mut glib::ffi::GError) -> *mut c_void;
        fn nm_vpn_editor_plugin_import(*mut c_void, *const c_char, *mut *mut glib::ffi::GError) -> *mut c_void;
        fn nm_conn_wireguard_import(*const c_char, *mut *mut glib::ffi::GError) -> *mut c_void;
        fn nm_connection_normalize(*mut c_void, *mut glib::ffi::GHashTable, *mut c_int, *mut *mut glib::ffi::GError) -> c_int;
        fn nm_connection_to_dbus(*mut c_void, c_uint) -> *mut glib::ffi::GVariant;
    }
}

static LIB: OnceLock<Result<LibNm, crate::dlib::DlError>> = OnceLock::new();
struct Object(*mut c_void);
impl Drop for Object {
    fn drop(&mut self) {
        unsafe {
            glib::gobject_ffi::g_object_unref(self.0.cast());
        }
    }
}

fn failure(lang: Lang, error: *mut glib::ffi::GError) -> ApiError {
    if error.is_null() {
        return ApiError::tr("bad_args", lang, "net.badImport", NO_PARAMS);
    }
    let error: glib::Error = unsafe { from_glib_full(error) };
    let detail: String = error
        .message()
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(300)
        .collect();
    ApiError::tr(
        "failed",
        lang,
        "net.importFailed",
        &json!({"error": detail}),
    )
}

fn object(
    pointer: *mut c_void,
    error: *mut glib::ffi::GError,
    lang: Lang,
) -> Result<Object, ApiError> {
    if pointer.is_null() {
        Err(failure(lang, error))
    } else {
        Ok(Object(pointer))
    }
}

/// plugin_file comes only from the installed .name inventory, not from a client-supplied library path.
/// libnm validates module ownership before loading the import plugin. The GTK editor factory is never called.
pub(super) fn parse(
    path: &str,
    kind: &str,
    plugin_file: Option<&Path>,
    lang: Lang,
) -> Result<Groups, ApiError> {
    let lib = LIB
        .get_or_init(LibNm::load)
        .as_ref()
        .map_err(|_| ApiError::tr("unsupported", lang, "net.noPlugin", NO_PARAMS))?;
    let path = CString::new(path).map_err(|_| failure(lang, std::ptr::null_mut()))?;
    let mut error = std::ptr::null_mut();
    let connection = if kind == "wireguard" {
        let pointer = unsafe { (lib.nm_conn_wireguard_import)(path.as_ptr(), &mut error) };
        object(pointer, error, lang)?
    } else {
        let file = plugin_file
            .and_then(|p| p.to_str())
            .and_then(|s| CString::new(s).ok())
            .ok_or_else(|| ApiError::tr("unsupported", lang, "net.noPlugin", NO_PARAMS))?;
        let pointer = unsafe { (lib.nm_vpn_plugin_info_new_from_file)(file.as_ptr(), &mut error) };
        let info = object(pointer, error, lang)?;
        // The plugin is borrowed from info and remains alive until the connection has been imported.
        let plugin = unsafe { (lib.nm_vpn_plugin_info_load_editor_plugin)(info.0, &mut error) };
        if plugin.is_null() {
            return Err(failure(lang, error));
        }
        let pointer =
            unsafe { (lib.nm_vpn_editor_plugin_import)(plugin, path.as_ptr(), &mut error) };
        object(pointer, error, lang)?
    };
    let normalized = unsafe {
        (lib.nm_connection_normalize)(
            connection.0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut error,
        )
    };
    if normalized == 0 {
        return Err(failure(lang, error));
    }
    // NM_CONNECTION_SERIALIZE_ALL is zero; it retains imported secrets for this private AddConnection2 call.
    let pointer = unsafe { (lib.nm_connection_to_dbus)(connection.0, 0) };
    if pointer.is_null() {
        return Err(failure(lang, std::ptr::null_mut()));
    }
    let value: glib::Variant = unsafe { from_glib_full(pointer) };
    if value.type_().as_str() != "a{sa{sv}}" {
        return Err(failure(lang, std::ptr::null_mut()));
    }
    Ok(settings_map::groups_of(&value))
}
