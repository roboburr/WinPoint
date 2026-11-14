//! The Wayland application ID of a window (native/shell/src/wayland.rs): the extension knows a window by it. GTK
//! sets it from the prgname (= the app id, kit::run) when the toplevel maps; set_app_id gives one window another
//! (Explorer's picker: io.github.win11shell.FilePicker), applied on every realize and map
//! (gdk_wayland_toplevel_set_application_id, GTK 4.10+). Elsewhere than Wayland it does nothing.
use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;

unsafe extern "C" {
    fn gdk_wayland_toplevel_set_application_id(toplevel: *mut gtk::gdk::ffi::GdkToplevel, id: *const libc::c_char);
}

/// Give the window's toplevel surface this application ID whenever it is realized or mapped (Wayland only).
pub fn set_app_id(window: &impl IsA<gtk::Window>, id: &'static str) {
    let apply = move |w: &gtk::Window| {
        let Some(surface) = w.surface() else { return };
        if !surface.display().type_().name().contains("Wayland") {
            return;
        }
        if let Ok(toplevel) = surface.downcast::<gtk::gdk::Toplevel>()
            && let Ok(id) = std::ffi::CString::new(id)
        {
            unsafe { gdk_wayland_toplevel_set_application_id(toplevel.to_glib_none().0, id.as_ptr()) };
        }
    };
    let w = window.upcast_ref::<gtk::Window>();
    if w.is_realized() {
        apply(w);
    }
    w.connect_realize(move |w| apply(w));
    w.connect_map(move |w| {
        let w: &gtk::Window = w.upcast_ref();
        apply(w);
    });
}
