//! sound.test: GNOME Settings' speaker test and alert preview through libcanberra (dlopen, its PulseAudio driver):
//! "audio-channel-<position>" forced onto that channel of the chosen sink (canberra.force_channel, media.role test),
//! as cc-speaker-test-button.c plays it, and an alert file for the Geluidsschema's Testen. One context per process,
//! with GNOME Settings' application.id, so its playback stays out of the Volume mixer like GNOME's own (client.rs
//! `METER_APP_ID`).
use libc::{c_char, c_int, c_void};
use std::ffi::CString;
use std::sync::{Mutex, OnceLock};

crate::dlib::dlib! {
    /// libcanberra 0.30
    pub(crate) struct Canberra ["libcanberra.so.0"] {
        fn ca_context_create(*mut *mut c_void) -> c_int;
        fn ca_context_set_driver(*mut c_void, *const c_char) -> c_int;
        fn ca_context_change_device(*mut c_void, *const c_char) -> c_int;
        fn ca_context_change_props_full(*mut c_void, *mut c_void) -> c_int;
        fn ca_context_open(*mut c_void) -> c_int;
        fn ca_context_play_full(*mut c_void, u32, *mut c_void, *const c_void, *mut c_void) -> c_int;
        fn ca_proplist_create(*mut *mut c_void) -> c_int;
        fn ca_proplist_destroy(*mut c_void) -> c_int;
        fn ca_proplist_sets(*mut c_void, *const c_char, *const c_char) -> c_int;
    }
}

/// CA_ERROR_NOTFOUND (canberra.h): the sound theme has no such event.
const NOT_FOUND: c_int = -9;

/// The channels GNOME's speaker test knows (pa_channel_position_to_string names).
pub(crate) const CHANNELS: &[&str] = &[
    "mono",
    "front-left",
    "front-right",
    "front-center",
    "rear-left",
    "rear-right",
    "rear-center",
    "lfe",
    "side-left",
    "side-right",
    "front-left-of-center",
    "front-right-of-center",
];

struct Player {
    lib: &'static Canberra,
    ctx: usize,
}

static LIB: OnceLock<Option<Canberra>> = OnceLock::new();
static PLAYER: Mutex<Option<Player>> = Mutex::new(None);

fn lib() -> Option<&'static Canberra> {
    LIB.get_or_init(|| Canberra::load().ok()).as_ref()
}

fn proplist(lib: &Canberra, entries: &[(&str, &str)]) -> Option<*mut c_void> {
    let mut pl: *mut c_void = std::ptr::null_mut();
    if unsafe { (lib.ca_proplist_create)(&mut pl) } < 0 || pl.is_null() {
        return None;
    }
    for (k, v) in entries {
        if let (Ok(k), Ok(v)) = (CString::new(*k), CString::new(*v)) {
            unsafe { (lib.ca_proplist_sets)(pl, k.as_ptr(), v.as_ptr()) };
        }
    }
    Some(pl)
}

/// Whether test sounds can play in this process (libcanberra loads; never in a test build).
pub(crate) fn available() -> bool {
    !cfg!(test) && lib().is_some()
}

/// What to play.
pub(crate) enum Sound<'a> {
    /// GNOME's speaker test on one channel ("front-left", ...).
    Channel(&'a str),
    /// A sound file (an alert of GNOME's alert sounds).
    File(&'a str),
    /// A theme event (the theme's own bell for Standaard).
    Event(&'a str),
}

/// Play `sound` on `device` (a sink name; None: the default) with the sound theme `theme`; false when it could not
/// start.
pub(crate) fn play(device: Option<&str>, theme: &str, sound: Sound) -> bool {
    if !available() {
        return false;
    }
    let Some(lib) = lib() else { return false };
    let mut guard = PLAYER.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        let mut ctx: *mut c_void = std::ptr::null_mut();
        if unsafe { (lib.ca_context_create)(&mut ctx) } < 0 || ctx.is_null() {
            return false;
        }
        unsafe { (lib.ca_context_set_driver)(ctx, c"pulse".as_ptr()) };
        if let Some(pl) = proplist(
            lib,
            &[
                ("application.name", "WinPoint"),
                ("application.id", super::client::METER_APP_ID),
            ],
        ) {
            unsafe {
                (lib.ca_context_change_props_full)(ctx, pl);
                (lib.ca_proplist_destroy)(pl);
            }
        }
        unsafe { (lib.ca_context_open)(ctx) };
        *guard = Some(Player {
            lib,
            ctx: ctx as usize,
        });
    }
    let Some(player) = guard.as_ref() else {
        return false;
    };
    let ctx = player.ctx as *mut c_void;
    let device = device.and_then(|d| CString::new(d).ok());
    unsafe {
        (player.lib.ca_context_change_device)(
            ctx,
            device.as_ref().map_or(std::ptr::null(), |d| d.as_ptr()),
        )
    };
    let attempt = |props: &[(&str, &str)]| -> c_int {
        let Some(pl) = proplist(player.lib, props) else {
            return -1;
        };
        let r = unsafe {
            (player.lib.ca_context_play_full)(ctx, 0, pl, std::ptr::null(), std::ptr::null_mut())
        };
        unsafe { (player.lib.ca_proplist_destroy)(pl) };
        r
    };
    match sound {
        Sound::Channel(channel) => {
            // audio-channel-<position>, else the test signal, else the bell (GNOME's fallbacks)
            for id in [
                format!("audio-channel-{channel}"),
                "audio-test-signal".into(),
                "bell".into(),
            ] {
                let r = attempt(&[
                    ("media.role", "test"),
                    ("media.name", "Speaker test"),
                    ("event.id", &id),
                    ("canberra.force_channel", channel),
                    ("canberra.enable", "1"),
                    ("canberra.xdg-theme.name", theme),
                ]);
                if r != NOT_FOUND {
                    return r >= 0;
                }
            }
            false
        }
        Sound::File(path) => {
            attempt(&[
                ("media.role", "event"),
                ("media.name", "Alert sound"),
                ("media.filename", path),
                ("canberra.enable", "1"),
            ]) >= 0
        }
        Sound::Event(id) => {
            attempt(&[
                ("media.role", "event"),
                ("event.id", id),
                ("canberra.enable", "1"),
                ("canberra.xdg-theme.name", theme),
            ]) >= 0
        }
    }
}
