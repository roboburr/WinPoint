//! GIO launch context that supplies the compositor's fresh activation token.
//!
//! Setting environment variables alone does not populate the platform data of a
//! DBusActivatable app. gio 0.18 has no AppLaunchContext subclass bindings, so a
//! small GObject subclass overrides only this virtual method. GIO retains all
//! desktop-entry parsing, environment handling and activation/fallback behavior.

use glib::{prelude::*, translate::*};
use std::{
    ffi::{CString, OsString},
    sync::OnceLock,
};

#[path = "launch_context/portal.rs"]
mod portal;

const TOKEN_KEY: &str = "win11-gnome-launch-token";

unsafe extern "C" fn startup_id(
    context: *mut gio::ffi::GAppLaunchContext,
    _info: *mut gio::ffi::GAppInfo,
    _files: *mut glib::ffi::GList,
) -> *mut libc::c_char {
    // The instance data is immutable and owned by this GObject until finalization.
    // Return a GLib allocation, as required by the virtual method's transfer-full ABI.
    let context: glib::translate::Borrowed<gio::AppLaunchContext> =
        unsafe { from_glib_borrow(context) };
    unsafe {
        context
            .data::<CString>(TOKEN_KEY)
            .map(|token| glib::ffi::g_strdup(token.as_ref().as_ptr()))
            .unwrap_or(std::ptr::null_mut())
    }
}

unsafe extern "C" fn class_init(class: glib::ffi::gpointer, _data: glib::ffi::gpointer) {
    // GType has already copied the parent class; preserve every other virtual method.
    unsafe {
        (*(class as *mut gio::ffi::GAppLaunchContextClass)).get_startup_notify_id =
            Some(startup_id);
    }
}

fn context_type() -> glib::Type {
    static TYPE: OnceLock<glib::Type> = OnceLock::new();
    *TYPE.get_or_init(|| unsafe {
        from_glib(glib::gobject_ffi::g_type_register_static_simple(
            gio::ffi::g_app_launch_context_get_type(),
            c"Win11GnomeActivationContext".as_ptr(),
            std::mem::size_of::<gio::ffi::GAppLaunchContextClass>() as u32,
            Some(class_init),
            std::mem::size_of::<gio::ffi::GAppLaunchContext>() as u32,
            None,
            0,
        ))
    })
}

pub fn new(token: Option<String>) -> gio::AppLaunchContext {
    use gio::prelude::*;
    let context = if let Some(token) = token.and_then(|token| CString::new(token).ok()) {
        let context = glib::Object::with_type(context_type())
            .downcast::<gio::AppLaunchContext>()
            .expect("activation context derives from GAppLaunchContext");
        unsafe { context.set_data(TOKEN_KEY, token) };
        context
    } else {
        gio::AppLaunchContext::new()
    };
    configure_portal_context(&context);
    context
}

pub(crate) fn configure_portal_context(context: &gio::AppLaunchContext) {
    use gio::prelude::*;
    if let Some(original) = portal::captured() {
        if let Some(original) = original {
            context.setenv("GTK_USE_PORTAL", original);
        } else {
            context.unsetenv("GTK_USE_PORTAL");
        }
        context.unsetenv("WIN11_PORTAL_ENV");
        context.unsetenv("WIN11_PORTAL_PARENT_GTK_USE_PORTAL");
    } else if portal::default_opt_in()
        && !context.environment().iter().any(|entry| {
            entry
                .to_str()
                .is_some_and(|value| value.starts_with("GTK_USE_PORTAL="))
        })
    {
        context.setenv("GTK_USE_PORTAL", "1");
    }
}
/// Restore the backend's local GTK override, or opt only this new child into the owned chooser.
pub(crate) fn configure_portal_child_environment(
    command: &mut std::process::Command,
    base: Option<&[(OsString, OsString)]>,
) {
    if let Some(original) = portal::captured() {
        if let Some(original) = original {
            command.env("GTK_USE_PORTAL", original);
        } else {
            command.env_remove("GTK_USE_PORTAL");
        }
        command.env_remove("WIN11_PORTAL_ENV");
        command.env_remove("WIN11_PORTAL_PARENT_GTK_USE_PORTAL");
    } else if !command.get_envs().any(|(key, _)| key == "GTK_USE_PORTAL")
        && !base.is_some_and(|env| env.iter().any(|(key, _)| key == "GTK_USE_PORTAL"))
        && portal::default_opt_in()
    {
        command.env("GTK_USE_PORTAL", "1");
    }
}
pub(crate) fn restore_portal_child_environment(command: &mut std::process::Command) {
    configure_portal_child_environment(command, None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use gio::prelude::*;

    #[test]
    fn virtual_startup_method_returns_own_token_for_gio_platform_data() {
        let info = gio::AppInfo::create_from_commandline(
            "/usr/bin/true",
            Some("Activation context fixture"),
            gio::AppInfoCreateFlags::NONE,
        )
        .unwrap();
        let first = new(Some("fresh-first-token".into()));
        let second = new(Some("fresh-second-token".into()));
        assert_eq!(
            first.startup_notify_id(&info, &[]).as_deref(),
            Some("fresh-first-token")
        );
        assert_eq!(
            second.startup_notify_id(&info, &[]).as_deref(),
            Some("fresh-second-token")
        );
        assert_eq!(
            first.startup_notify_id(&info, &[]).as_deref(),
            Some("fresh-first-token")
        );
        assert_eq!(new(None).startup_notify_id(&info, &[]), None);
        assert_eq!(
            new(Some("invalid\0token".into())).startup_notify_id(&info, &[]),
            None
        );
    }
}
