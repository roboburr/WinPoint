//! xdg-foreign and X11 window identifiers. All calls run on the GTK thread.
use glib::translate::{ToGlibPtr, from_glib_full, from_glib_none};
use gtk::{gdk, prelude::*};
use std::ffi::CString;
pub(super) struct Parent {
    pub display: gdk::Display,
    kind: Kind,
}
enum Kind {
    None,
    Wayland(CString),
    X11(gdk::Window),
}
impl Parent {
    pub fn new(handle: &str) -> Result<Self, String> {
        if handle.is_empty() {
            return Ok(Self {
                display: gdk::Display::default().ok_or("No display")?,
                kind: Kind::None,
            });
        }
        if let Some(value) = handle.strip_prefix("wayland:") {
            if value.is_empty() || value.len() > 1024 {
                return Err("Invalid Wayland parent".into());
            }
            let display = display("wayland")?;
            let value = CString::new(value).map_err(|_| "Invalid Wayland parent")?;
            return Ok(Self {
                display,
                kind: Kind::Wayland(value),
            });
        }
        if let Some(value) = handle.strip_prefix("x11:") {
            let value = value.strip_prefix("0x").unwrap_or(value);
            if value.is_empty() || value.len() > 16 || !value.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("Invalid X11 parent".into());
            }
            let xid = u64::from_str_radix(value, 16)
                .ok()
                .filter(|x| *x > 0 && *x <= u32::MAX as u64)
                .ok_or("Invalid X11 parent")?;
            let display = display("x11")?;
            let raw = unsafe {
                gdk_x11_window_foreign_new_for_display(
                    display.to_glib_none().0,
                    xid as libc::c_ulong,
                )
            };
            if raw.is_null() {
                return Err("X11 parent is unavailable".into());
            }
            let window = unsafe { from_glib_full(raw) };
            return Ok(Self {
                display,
                kind: Kind::X11(window),
            });
        }
        Err("Unknown parent-window identifier".into())
    }
    pub fn apply(&self, window: &gtk::ApplicationWindow) -> Result<(), String> {
        let child = window.window().ok_or("Picker surface is not realized")?;
        match &self.kind {
            Kind::None => Ok(()),
            Kind::Wayland(handle) => {
                if unsafe {
                    gdk_wayland_window_set_transient_for_exported(
                        child.to_glib_none().0,
                        handle.as_ptr() as *mut _,
                    )
                } != 0
                {
                    Ok(())
                } else {
                    Err("Wayland parent could not be imported".into())
                }
            }
            Kind::X11(parent) => {
                child.set_transient_for(parent);
                Ok(())
            }
        }
    }
}
fn display(kind: &str) -> Result<gdk::Display, String> {
    for display in gdk::DisplayManager::get().list_displays() {
        let ty = display.type_().name();
        if (kind == "x11" && ty == "GdkX11Display")
            || (kind == "wayland" && ty == "GdkWaylandDisplay")
        {
            return Ok(display);
        }
    }
    // Like the GTK reference backend, open the requested backend when a caller uses the other window system.
    let allowed = CString::new(kind).unwrap();
    let raw = unsafe {
        gdk_set_allowed_backends(allowed.as_ptr());
        let value = gdk_display_open(std::ptr::null());
        gdk_set_allowed_backends(std::ptr::null());
        value
    };
    if raw.is_null() {
        Err(format!("{kind} display is unavailable"))
    } else {
        Ok(unsafe { from_glib_none(raw) })
    }
}
#[link(name = "gdk-3")]
unsafe extern "C" {
    fn gdk_wayland_window_set_transient_for_exported(
        window: *mut gdk::ffi::GdkWindow,
        handle: *mut libc::c_char,
    ) -> libc::c_int;
    fn gdk_x11_window_foreign_new_for_display(
        display: *mut gdk::ffi::GdkDisplay,
        xid: libc::c_ulong,
    ) -> *mut gdk::ffi::GdkWindow;
    fn gdk_set_allowed_backends(backends: *const libc::c_char);
    fn gdk_display_open(name: *const libc::c_char) -> *mut gdk::ffi::GdkDisplay;
}
