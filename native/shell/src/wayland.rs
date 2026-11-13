//! Per-window Wayland application ID: the extension tells our surfaces apart by it (Panel, Start, Desktop),
//! and they all live in one process. GTK 4 has gdk_wayland_toplevel_set_application_id (4.10+) for that.
//!
//! GTK creates the xdg_toplevel only when the window maps (and again on every show after a hide), with the
//! process's prgname (PANEL_ID) as its app ID; the call is a no-op before that. So the ID is applied on every map
//! as well; mutter updates wm-class at once and the extension re-checks the window on notify::wm-class.
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

/// Keep a full-monitor surface (taskbar, desktop) at `size()` whatever the compositor's bounds say. GTK 4 clamps
/// a toplevel's own size to the configure bounds (the work area: monitor minus the top bar and our own strut), so
/// without this the panel window ends 48 px short of the bottom edge and the strip floats above it. Our
/// compute-size handler runs after GTK's (connected at realize) and has the last word.
pub fn keep_size(window: &impl IsA<gtk::Window>, size: impl Fn() -> (i32, i32) + 'static) {
    let size = std::rc::Rc::new(size);
    let hook = move |w: &gtk::Window| {
        let Some(surface) = w.surface() else { return };
        let Ok(toplevel) = surface.downcast::<gtk::gdk::Toplevel>() else { return };
        let size = size.clone();
        toplevel.connect_compute_size(move |_, s| {
            let (width, height) = size();
            if width > 0 && height > 0 {
                s.set_size(width, height);
            }
        });
    };
    let w = window.upcast_ref::<gtk::Window>();
    if w.is_realized() {
        hook(w);
    }
    w.connect_realize(move |w| hook(w));
}
