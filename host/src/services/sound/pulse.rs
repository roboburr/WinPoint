//! libpulse 17 (the PulseAudio protocol that pipewire-pulse speaks, as GNOME Settings' libgvc does) and its GLib
//! main loop, through dlopen (host/src/dlib.rs; amendment 39). Only the parts the Sound service uses: the context,
//! the introspection of sinks, sources, streams, clients and cards, the writes, ext-stream-restore (the event
//! role's volume), peak-detect record streams, and the volume helpers GNOME's balance, fade and subwoofer sliders
//! use (pa_cvolume_set_balance and friends).
//!
//! The struct layouts are those of <pulse/introspect.h>, <pulse/sample.h>, <pulse/channelmap.h>, <pulse/volume.h>
//! and <pulse/ext-stream-restore.h> of libpulse 17 on x86_64; `tests.rs` holds every size and the offsets of the
//! fields read, as a C program with offsetof() printed them, so a wrong field fails a test instead of reading
//! garbage.
#![allow(dead_code)] // the full layouts: some fields only keep the offsets right
use libc::{c_char, c_int, c_void};
use std::sync::OnceLock;

pub(crate) const CHANNELS_MAX: usize = 32;
/// PA_VOLUME_NORM: 100 %.
pub(crate) const VOLUME_NORM: u32 = 0x10000;
pub(crate) const INVALID_INDEX: u32 = u32::MAX;
/// PA_SAMPLE_FLOAT32NE on a little-endian machine (PA_SAMPLE_FLOAT32LE).
pub(crate) const SAMPLE_FLOAT32NE: c_int = 5;
/// pa_context_flags_t
pub(crate) const CONTEXT_NOAUTOSPAWN: c_int = 1;
/// pa_context_state_t
pub(crate) const CONTEXT_READY: c_int = 4;
pub(crate) const CONTEXT_FAILED: c_int = 5;
pub(crate) const CONTEXT_TERMINATED: c_int = 6;
/// pa_stream_state_t
pub(crate) const STREAM_FAILED: c_int = 3;
pub(crate) const STREAM_TERMINATED: c_int = 4;
/// pa_stream_flags_t
pub(crate) const STREAM_DONT_MOVE: c_int = 0x200;
pub(crate) const STREAM_PEAK_DETECT: c_int = 0x800;
pub(crate) const STREAM_ADJUST_LATENCY: c_int = 0x2000;
/// pa_subscription_mask_t: sink, source, sink input, source output, client, server, card
pub(crate) const SUBSCRIBE_MASK: c_int = 1 | 2 | 4 | 8 | 32 | 128 | 512;
/// pa_subscription_event_type_t
pub(crate) const FACILITY_MASK: c_int = 0x0f;
pub(crate) const TYPE_MASK: c_int = 0x30;
pub(crate) const EVENT_REMOVE: c_int = 0x20;
pub(crate) const FACILITY_SINK: c_int = 0;
pub(crate) const FACILITY_SOURCE: c_int = 1;
pub(crate) const FACILITY_SINK_INPUT: c_int = 2;
pub(crate) const FACILITY_SOURCE_OUTPUT: c_int = 3;
pub(crate) const FACILITY_CLIENT: c_int = 5;
pub(crate) const FACILITY_SERVER: c_int = 7;
pub(crate) const FACILITY_CARD: c_int = 9;
/// pa_port_available_t
pub(crate) const PORT_AVAILABLE_NO: c_int = 1;
pub(crate) const PORT_AVAILABLE_YES: c_int = 2;
/// pa_direction_t
pub(crate) const DIRECTION_OUTPUT: c_int = 1;
pub(crate) const DIRECTION_INPUT: c_int = 2;
/// pa_update_mode_t
pub(crate) const UPDATE_REPLACE: c_int = 2;
/// pa_channel_position_t
pub(crate) const POSITION_MONO: c_int = 0;
pub(crate) const POSITION_LFE: c_int = 7;
/// pa_sink_flags_t / pa_source_flags_t
pub(crate) const FLAG_HW_VOLUME_CTRL: c_int = 0x1;
pub(crate) const FLAG_DECIBEL_VOLUME: c_int = 0x20;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct SampleSpec {
    pub format: c_int,
    pub rate: u32,
    pub channels: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct ChannelMap {
    pub channels: u8,
    pub map: [c_int; CHANNELS_MAX],
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct CVolume {
    pub channels: u8,
    pub values: [u32; CHANNELS_MAX],
}

impl ChannelMap {
    pub(crate) fn of(positions: &[i32]) -> ChannelMap {
        let mut m = ChannelMap {
            channels: positions.len().min(CHANNELS_MAX) as u8,
            map: [0; CHANNELS_MAX],
        };
        for (i, p) in positions.iter().take(CHANNELS_MAX).enumerate() {
            m.map[i] = *p;
        }
        m
    }

    pub(crate) fn positions(&self) -> Vec<i32> {
        self.map[..(self.channels as usize).min(CHANNELS_MAX)].to_vec()
    }
}

impl CVolume {
    pub(crate) fn of(values: &[u32]) -> CVolume {
        let mut v = CVolume {
            channels: values.len().min(CHANNELS_MAX) as u8,
            values: [0; CHANNELS_MAX],
        };
        for (i, x) in values.iter().take(CHANNELS_MAX).enumerate() {
            v.values[i] = *x;
        }
        v
    }

    pub(crate) fn values(&self) -> Vec<u32> {
        self.values[..(self.channels as usize).min(CHANNELS_MAX)].to_vec()
    }
}

#[repr(C)]
pub(crate) struct BufferAttr {
    pub maxlength: u32,
    pub tlength: u32,
    pub prebuf: u32,
    pub minreq: u32,
    pub fragsize: u32,
}

#[repr(C)]
pub(crate) struct ServerInfo {
    pub user_name: *const c_char,
    pub host_name: *const c_char,
    pub server_version: *const c_char,
    pub server_name: *const c_char,
    pub sample_spec: SampleSpec,
    pub default_sink_name: *const c_char,
    pub default_source_name: *const c_char,
    pub cookie: u32,
    pub channel_map: ChannelMap,
}

/// pa_sink_port_info and pa_source_port_info (the same layout).
#[repr(C)]
pub(crate) struct PortInfo {
    pub name: *const c_char,
    pub description: *const c_char,
    pub priority: u32,
    pub available: c_int,
    pub availability_group: *const c_char,
    pub type_: u32,
}

/// pa_sink_info and pa_source_info (the same layout: `monitor` is a sink's monitor_source / a source's
/// monitor_of_sink, `monitor_name` likewise).
#[repr(C)]
pub(crate) struct DeviceInfo {
    pub name: *const c_char,
    pub index: u32,
    pub description: *const c_char,
    pub sample_spec: SampleSpec,
    pub channel_map: ChannelMap,
    pub owner_module: u32,
    pub volume: CVolume,
    pub mute: c_int,
    pub monitor: u32,
    pub monitor_name: *const c_char,
    pub latency: u64,
    pub driver: *const c_char,
    pub flags: c_int,
    pub proplist: *mut c_void,
    pub configured_latency: u64,
    pub base_volume: u32,
    pub state: c_int,
    pub n_volume_steps: u32,
    pub card: u32,
    pub n_ports: u32,
    pub ports: *mut *mut PortInfo,
    pub active_port: *mut PortInfo,
    pub n_formats: u8,
    pub formats: *mut *mut c_void,
}

#[repr(C)]
pub(crate) struct SinkInputInfo {
    pub index: u32,
    pub name: *const c_char,
    pub owner_module: u32,
    pub client: u32,
    pub sink: u32,
    pub sample_spec: SampleSpec,
    pub channel_map: ChannelMap,
    pub volume: CVolume,
    pub buffer_usec: u64,
    pub sink_usec: u64,
    pub resample_method: *const c_char,
    pub driver: *const c_char,
    pub mute: c_int,
    pub proplist: *mut c_void,
    pub corked: c_int,
    pub has_volume: c_int,
    pub volume_writable: c_int,
    pub format: *mut c_void,
}

#[repr(C)]
pub(crate) struct SourceOutputInfo {
    pub index: u32,
    pub name: *const c_char,
    pub owner_module: u32,
    pub client: u32,
    pub source: u32,
    pub sample_spec: SampleSpec,
    pub channel_map: ChannelMap,
    pub buffer_usec: u64,
    pub source_usec: u64,
    pub resample_method: *const c_char,
    pub driver: *const c_char,
    pub proplist: *mut c_void,
    pub corked: c_int,
    pub volume: CVolume,
    pub mute: c_int,
    pub has_volume: c_int,
    pub volume_writable: c_int,
    pub format: *mut c_void,
}

#[repr(C)]
pub(crate) struct ClientInfo {
    pub index: u32,
    pub name: *const c_char,
    pub owner_module: u32,
    pub driver: *const c_char,
    pub proplist: *mut c_void,
}

#[repr(C)]
pub(crate) struct CardProfileInfo2 {
    pub name: *const c_char,
    pub description: *const c_char,
    pub n_sinks: u32,
    pub n_sources: u32,
    pub priority: u32,
    pub available: c_int,
}

#[repr(C)]
pub(crate) struct CardPortInfo {
    pub name: *const c_char,
    pub description: *const c_char,
    pub priority: u32,
    pub available: c_int,
    pub direction: c_int,
    pub n_profiles: u32,
    pub profiles: *mut *mut c_void,
    pub proplist: *mut c_void,
    pub latency_offset: i64,
    pub profiles2: *mut *mut CardProfileInfo2,
    pub availability_group: *const c_char,
    pub type_: u32,
}

#[repr(C)]
pub(crate) struct CardInfo {
    pub index: u32,
    pub name: *const c_char,
    pub owner_module: u32,
    pub driver: *const c_char,
    pub n_profiles: u32,
    pub profiles: *mut c_void,
    pub active_profile: *mut c_void,
    pub proplist: *mut c_void,
    pub n_ports: u32,
    pub ports: *mut *mut CardPortInfo,
    pub profiles2: *mut *mut CardProfileInfo2,
    pub active_profile2: *mut CardProfileInfo2,
}

#[repr(C)]
pub(crate) struct ExtStreamRestoreInfo {
    pub name: *const c_char,
    pub channel_map: ChannelMap,
    pub volume: CVolume,
    pub device: *const c_char,
    pub mute: c_int,
}

pub(crate) type NotifyCb = Option<unsafe extern "C" fn(*mut c_void, *mut c_void)>;
pub(crate) type SubscribeCb = Option<unsafe extern "C" fn(*mut c_void, c_int, u32, *mut c_void)>;
pub(crate) type SuccessCb = Option<unsafe extern "C" fn(*mut c_void, c_int, *mut c_void)>;
pub(crate) type ServerInfoCb =
    Option<unsafe extern "C" fn(*mut c_void, *const ServerInfo, *mut c_void)>;
pub(crate) type DeviceInfoCb =
    Option<unsafe extern "C" fn(*mut c_void, *const DeviceInfo, c_int, *mut c_void)>;
pub(crate) type SinkInputInfoCb =
    Option<unsafe extern "C" fn(*mut c_void, *const SinkInputInfo, c_int, *mut c_void)>;
pub(crate) type SourceOutputInfoCb =
    Option<unsafe extern "C" fn(*mut c_void, *const SourceOutputInfo, c_int, *mut c_void)>;
pub(crate) type ClientInfoCb =
    Option<unsafe extern "C" fn(*mut c_void, *const ClientInfo, c_int, *mut c_void)>;
pub(crate) type CardInfoCb =
    Option<unsafe extern "C" fn(*mut c_void, *const CardInfo, c_int, *mut c_void)>;
pub(crate) type RestoreReadCb =
    Option<unsafe extern "C" fn(*mut c_void, *const ExtStreamRestoreInfo, c_int, *mut c_void)>;
pub(crate) type StreamRequestCb = Option<unsafe extern "C" fn(*mut c_void, usize, *mut c_void)>;

crate::dlib::dlib! {
    /// libpulse 17 (pipewire-pulse speaks its protocol).
    pub(crate) struct Pulse ["libpulse.so.0"] {
        fn pa_context_new_with_proplist(*mut c_void, *const c_char, *mut c_void) -> *mut c_void;
        fn pa_context_unref(*mut c_void) -> ();
        fn pa_context_connect(*mut c_void, *const c_char, c_int, *const c_void) -> c_int;
        fn pa_context_disconnect(*mut c_void) -> ();
        fn pa_context_get_state(*const c_void) -> c_int;
        fn pa_context_errno(*const c_void) -> c_int;
        fn pa_strerror(c_int) -> *const c_char;
        fn pa_context_set_state_callback(*mut c_void, NotifyCb, *mut c_void) -> ();
        fn pa_context_set_subscribe_callback(*mut c_void, SubscribeCb, *mut c_void) -> ();
        fn pa_context_subscribe(*mut c_void, c_int, SuccessCb, *mut c_void) -> *mut c_void;
        fn pa_context_get_server_info(*mut c_void, ServerInfoCb, *mut c_void) -> *mut c_void;
        fn pa_context_get_sink_info_list(*mut c_void, DeviceInfoCb, *mut c_void) -> *mut c_void;
        fn pa_context_get_sink_info_by_index(*mut c_void, u32, DeviceInfoCb, *mut c_void) -> *mut c_void;
        fn pa_context_get_source_info_list(*mut c_void, DeviceInfoCb, *mut c_void) -> *mut c_void;
        fn pa_context_get_source_info_by_index(*mut c_void, u32, DeviceInfoCb, *mut c_void) -> *mut c_void;
        fn pa_context_get_sink_input_info_list(*mut c_void, SinkInputInfoCb, *mut c_void) -> *mut c_void;
        fn pa_context_get_sink_input_info(*mut c_void, u32, SinkInputInfoCb, *mut c_void) -> *mut c_void;
        fn pa_context_get_source_output_info_list(*mut c_void, SourceOutputInfoCb, *mut c_void) -> *mut c_void;
        fn pa_context_get_source_output_info(*mut c_void, u32, SourceOutputInfoCb, *mut c_void) -> *mut c_void;
        fn pa_context_get_client_info_list(*mut c_void, ClientInfoCb, *mut c_void) -> *mut c_void;
        fn pa_context_get_client_info(*mut c_void, u32, ClientInfoCb, *mut c_void) -> *mut c_void;
        fn pa_context_get_card_info_list(*mut c_void, CardInfoCb, *mut c_void) -> *mut c_void;
        fn pa_context_get_card_info_by_index(*mut c_void, u32, CardInfoCb, *mut c_void) -> *mut c_void;
        fn pa_context_set_default_sink(*mut c_void, *const c_char, SuccessCb, *mut c_void) -> *mut c_void;
        fn pa_context_set_default_source(*mut c_void, *const c_char, SuccessCb, *mut c_void) -> *mut c_void;
        fn pa_context_set_sink_volume_by_index(*mut c_void, u32, *const CVolume, SuccessCb, *mut c_void) -> *mut c_void;
        fn pa_context_set_source_volume_by_index(*mut c_void, u32, *const CVolume, SuccessCb, *mut c_void) -> *mut c_void;
        fn pa_context_set_sink_mute_by_index(*mut c_void, u32, c_int, SuccessCb, *mut c_void) -> *mut c_void;
        fn pa_context_set_source_mute_by_index(*mut c_void, u32, c_int, SuccessCb, *mut c_void) -> *mut c_void;
        fn pa_context_set_sink_input_volume(*mut c_void, u32, *const CVolume, SuccessCb, *mut c_void) -> *mut c_void;
        fn pa_context_set_sink_input_mute(*mut c_void, u32, c_int, SuccessCb, *mut c_void) -> *mut c_void;
        fn pa_context_set_source_output_volume(*mut c_void, u32, *const CVolume, SuccessCb, *mut c_void) -> *mut c_void;
        fn pa_context_set_source_output_mute(*mut c_void, u32, c_int, SuccessCb, *mut c_void) -> *mut c_void;
        fn pa_context_set_card_profile_by_index(*mut c_void, u32, *const c_char, SuccessCb, *mut c_void) -> *mut c_void;
        fn pa_context_set_sink_port_by_index(*mut c_void, u32, *const c_char, SuccessCb, *mut c_void) -> *mut c_void;
        fn pa_context_set_source_port_by_index(*mut c_void, u32, *const c_char, SuccessCb, *mut c_void) -> *mut c_void;
        fn pa_context_move_sink_input_by_index(*mut c_void, u32, u32, SuccessCb, *mut c_void) -> *mut c_void;
        fn pa_context_move_source_output_by_index(*mut c_void, u32, u32, SuccessCb, *mut c_void) -> *mut c_void;
        fn pa_operation_unref(*mut c_void) -> ();
        fn pa_proplist_new() -> *mut c_void;
        fn pa_proplist_free(*mut c_void) -> ();
        fn pa_proplist_sets(*mut c_void, *const c_char, *const c_char) -> c_int;
        fn pa_proplist_gets(*const c_void, *const c_char) -> *const c_char;
        fn pa_cvolume_max(*const CVolume) -> u32;
        fn pa_cvolume_scale(*mut CVolume, u32) -> *mut CVolume;
        fn pa_cvolume_get_balance(*const CVolume, *const ChannelMap) -> f32;
        fn pa_cvolume_set_balance(*mut CVolume, *const ChannelMap, f32) -> *mut CVolume;
        fn pa_cvolume_get_fade(*const CVolume, *const ChannelMap) -> f32;
        fn pa_cvolume_set_fade(*mut CVolume, *const ChannelMap, f32) -> *mut CVolume;
        fn pa_cvolume_get_position(*const CVolume, *const ChannelMap, c_int) -> u32;
        fn pa_cvolume_set_position(*mut CVolume, *const ChannelMap, c_int, u32) -> *mut CVolume;
        fn pa_channel_map_can_balance(*const ChannelMap) -> c_int;
        fn pa_channel_map_can_fade(*const ChannelMap) -> c_int;
        fn pa_channel_map_has_position(*const ChannelMap, c_int) -> c_int;
        fn pa_channel_position_to_string(c_int) -> *const c_char;
        fn pa_sw_volume_from_dB(f64) -> u32;
        fn pa_ext_stream_restore_read(*mut c_void, RestoreReadCb, *mut c_void) -> *mut c_void;
        fn pa_ext_stream_restore_write(*mut c_void, c_int, *const ExtStreamRestoreInfo, u32, c_int, SuccessCb, *mut c_void) -> *mut c_void;
        fn pa_ext_stream_restore_subscribe(*mut c_void, c_int, SuccessCb, *mut c_void) -> *mut c_void;
        fn pa_ext_stream_restore_set_subscribe_cb(*mut c_void, NotifyCb, *mut c_void) -> ();
        fn pa_stream_new_with_proplist(*mut c_void, *const c_char, *const SampleSpec, *const ChannelMap, *mut c_void) -> *mut c_void;
        fn pa_stream_set_read_callback(*mut c_void, StreamRequestCb, *mut c_void) -> ();
        fn pa_stream_set_state_callback(*mut c_void, NotifyCb, *mut c_void) -> ();
        fn pa_stream_connect_record(*mut c_void, *const c_char, *const BufferAttr, c_int) -> c_int;
        fn pa_stream_peek(*mut c_void, *mut *const c_void, *mut usize) -> c_int;
        fn pa_stream_drop(*mut c_void) -> c_int;
        fn pa_stream_disconnect(*mut c_void) -> c_int;
        fn pa_stream_unref(*mut c_void) -> ();
        fn pa_stream_get_state(*const c_void) -> c_int;
    }
}

crate::dlib::dlib! {
    /// libpulse's GLib main loop: pulse's events on a GMainContext of our own (amendment 39).
    pub(crate) struct PulseGlib ["libpulse-mainloop-glib.so.0"] {
        fn pa_glib_mainloop_new(*mut c_void) -> *mut c_void;
        fn pa_glib_mainloop_free(*mut c_void) -> ();
        fn pa_glib_mainloop_get_api(*mut c_void) -> *mut c_void;
    }
}

/// Both libraries, loaded once per process; None when either is missing (the Sound service then reports
/// unavailable and system.audio.* keeps wpctl).
pub(crate) struct Libs {
    pub pa: Pulse,
    pub glib: PulseGlib,
}

static LIBS: OnceLock<Option<Libs>> = OnceLock::new();

pub(crate) fn libs() -> Option<&'static Libs> {
    LIBS.get_or_init(|| {
        Some(Libs {
            pa: Pulse::load().ok()?,
            glib: PulseGlib::load().ok()?,
        })
    })
    .as_ref()
}

/// A C string of libpulse (NULL → "").
pub(crate) fn text(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe { std::ffi::CStr::from_ptr(p) }
        .to_string_lossy()
        .into_owned()
}

/// A C string of libpulse, or None for NULL.
pub(crate) fn opt_text(p: *const c_char) -> Option<String> {
    (!p.is_null()).then(|| text(p))
}

/// The proplist keys the model keeps (device icons and form factors, the bluez address and PipeWire's object id
/// for system.audio's wpctl-compatible ids, the application of a stream).
pub(crate) const PROPS: &[&str] = &[
    "device.description",
    "device.icon_name",
    "device.form_factor",
    "device.bus",
    "device.class",
    "device.api",
    "api.bluez5.address",
    "object.id",
    "node.name",
    "media.class",
    "media.name",
    "media.role",
    "media.icon_name",
    "application.name",
    "application.id",
    "application.icon_name",
    "application.process.binary",
    "application.process.id",
    "window.icon_name",
];

/// The kept keys of a proplist.
pub(crate) fn props(
    lib: &Pulse,
    list: *const c_void,
) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    if list.is_null() {
        return out;
    }
    for key in PROPS {
        let Ok(k) = std::ffi::CString::new(*key) else {
            continue;
        };
        let v = unsafe { (lib.pa_proplist_gets)(list, k.as_ptr()) };
        if !v.is_null() {
            out.insert((*key).to_owned(), text(v));
        }
    }
    out
}
