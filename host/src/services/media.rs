//! Original MPRIS payload and playback control, without a Python process.
use super::{ServiceContext, system::common::*};
use crate::files::{ApiError, ApiResult};
use gio::prelude::*;

use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const IFACE: &str = "org.mpris.MediaPlayer2.Player";
pub(crate) struct Media {
    ctx: Arc<ServiceContext>,
    stopped: AtomicBool,
    last: Mutex<Value>,
    dirty: Mutex<Option<Instant>>,
}
impl Media {
    pub(crate) fn new(ctx: Arc<ServiceContext>) -> Result<Arc<Self>, String> {
        let svc = Arc::new(Self {
            ctx,
            stopped: AtomicBool::new(false),
            last: Mutex::new(Value::Null),
            dirty: Mutex::new(None),
        });
        let weak = Arc::downgrade(&svc);
        thread::spawn(move || {
            let context = glib::MainContext::new();
            let _ = context.with_thread_default(|| {
                let bus = session().ok();
                let mut subscriptions = Vec::new();
                if let Some(bus) = bus.as_ref() {
                    let signal_specs = [
                        (
                            None,
                            "org.freedesktop.DBus.Properties",
                            "PropertiesChanged",
                            PATH,
                            None,
                            gio::DBusSignalFlags::NONE,
                        ),
                        (
                            Some("org.freedesktop.DBus"),
                            "org.freedesktop.DBus",
                            "NameOwnerChanged",
                            "/org/freedesktop/DBus",
                            Some("org.mpris.MediaPlayer2"),
                            gio::DBusSignalFlags::MATCH_ARG0_NAMESPACE,
                        ),
                        (
                            None,
                            IFACE,
                            "Seeked",
                            PATH,
                            None,
                            gio::DBusSignalFlags::NONE,
                        ),
                    ];
                    for (sender, iface, signal, path, arg0, flags) in signal_specs {
                        let weak = weak.clone();
                        subscriptions.push(bus.signal_subscribe(
                            sender,
                            Some(iface),
                            Some(signal),
                            Some(path),
                            arg0,
                            flags,
                            move |_, _, _, _, _, _| {
                                if let Some(service) = weak.upgrade() {
                                    service.changed();
                                }
                            },
                        ));
                    }
                }
                loop {
                    while context.pending() {
                        context.iteration(false);
                    }
                    let Some(service) = weak.upgrade() else { break };
                    if service.stopped.load(Ordering::Relaxed) {
                        break;
                    }
                    let ready = service
                        .dirty
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .is_some_and(|at| at.elapsed() >= Duration::from_millis(250));
                    if ready {
                        service
                            .dirty
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .take();
                        let players = service.players();
                        let mut key = players.clone();
                        if let Some(a) = key.as_array_mut() {
                            for p in a {
                                if let Some(p) = p.as_object_mut() {
                                    p.remove("updated");
                                    p.remove("position");
                                }
                            }
                        }
                        let mut last = service.last.lock().unwrap_or_else(|e| e.into_inner());
                        if *last != key {
                            *last = key;
                            drop(last);
                            service
                                .ctx
                                .emit("media.changed", json!({"players":players}));
                        }
                    }
                    drop(service);
                    thread::sleep(Duration::from_millis(10));
                }
                if let Some(bus) = bus {
                    for sub in subscriptions {
                        bus.signal_unsubscribe(sub);
                    }
                }
            });
        });
        Ok(svc)
    }
    pub(crate) fn shutdown(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }
    pub(crate) fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
        match method {
            "media.list" => Some(Ok(self.players())),
            "media.control" => Some(self.control(args)),
            _ => None,
        }
    }
    fn changed(&self) {
        *self.dirty.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    }
    fn names(&self) -> Vec<String> {
        let Ok(bus) = session() else { return vec![] };
        let Ok(v) = call(
            &bus,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "ListNames",
            None,
            1500,
        ) else {
            return vec![];
        };
        let mut names = first(&v)
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|v| {
                v.as_str()
                    .filter(|n| n.starts_with(PREFIX) && *n != "org.mpris.MediaPlayer2.playerctld")
                    .map(str::to_owned)
            })
            .collect::<Vec<_>>();
        names.sort();
        names
    }
    fn players(&self) -> Value {
        let Ok(bus) = session() else { return json!([]) };
        let mut out = Vec::new();
        for name in self.names() {
            let Ok(p) = getall(&bus, &name, PATH, IFACE) else {
                continue;
            };
            if p.as_object().is_none_or(|v| v.is_empty()) {
                continue;
            }
            let root = getall(&bus, &name, PATH, "org.mpris.MediaPlayer2").unwrap_or(json!({}));
            let mut player = player_dict(&name, &root, &p);
            // Reuse the same native application catalogue as Start and Explorer.
            if let Ok(catalogue) = self.ctx.call("apps.list", &json!({})) {
                if let Some(app) = catalogue
                    .as_array()
                    .and_then(|apps| apps.iter().find(|app| app["id"] == player["desktopId"]))
                {
                    if !app["icon"].is_null() {
                        player["icon"] = app["icon"].clone();
                    }
                }
            }
            out.push(player);
        }
        out.sort_by(|a, b| {
            let rank = |p: &Value| match p["status"].as_str() {
                Some("playing") => 0,
                Some("paused") => 1,
                _ => 2,
            };
            rank(a).cmp(&rank(b)).then_with(|| {
                string(&a["identity"])
                    .to_lowercase()
                    .cmp(&string(&b["identity"]).to_lowercase())
            })
        });
        json!(out)
    }
    fn control(&self, args: &Value) -> ApiResult {
        let method = match args["action"].as_str() {
            Some("playPause") => "PlayPause",
            Some("next") => "Next",
            Some("previous") => "Previous",
            Some("play") => "Play",
            Some("pause") => "Pause",
            Some("stop") => "Stop",
            _ => {
                return Err(ApiError::tr(
                    "bad_args",
                    self.ctx.lang(),
                    "media.unknownControl",
                    crate::i18n::NO_PARAMS,
                ));
            }
        };
        let id = args["id"]
            .as_str()
            .filter(|s| s.starts_with(PREFIX))
            .ok_or_else(|| {
                ApiError::tr(
                    "bad_args",
                    self.ctx.lang(),
                    "media.unknownPlayer",
                    crate::i18n::NO_PARAMS,
                )
            })?;
        if !self.names().iter().any(|n| n == id) {
            return Err(ApiError::tr(
                "not_found",
                self.ctx.lang(),
                "media.playerClosed",
                crate::i18n::NO_PARAMS,
            ));
        }
        if self.ctx.dry_run {
            return Ok(json!({"ok":true,"dryRun":true}));
        }
        let bus = session().map_err(|_| {
            ApiError::tr(
                "failed",
                self.ctx.lang(),
                "media.notResponding",
                crate::i18n::NO_PARAMS,
            )
        })?;
        call(&bus, id, PATH, IFACE, method, None, 2000).map_err(|_| {
            ApiError::tr(
                "failed",
                self.ctx.lang(),
                "media.notResponding",
                crate::i18n::NO_PARAMS,
            )
        })?;
        self.changed();
        Ok(json!({"ok":true}))
    }
}
pub(super) fn player_dict(name: &str, root: &Value, p: &Value) -> Value {
    let md = &p["Metadata"];
    let status = match p["PlaybackStatus"]
        .as_str()
        .unwrap_or("Stopped")
        .to_lowercase()
        .as_str()
    {
        "playing" => "playing",
        "paused" => "paused",
        _ => "stopped",
    };
    let art = string(&md["mpris:artUrl"]);
    let art_path = url::Url::parse(&art)
        .ok()
        .filter(|u| u.scheme() == "file")
        .and_then(|u| u.to_file_path().ok())
        .filter(|p| p.is_file());
    let label = string(&root["Identity"]);
    let short = name
        .strip_prefix(PREFIX)
        .unwrap_or(name)
        .split('.')
        .next()
        .unwrap_or("");
    let identity = if label.is_empty() {
        let mut chars = short.chars();
        chars
            .next()
            .map(|c| c.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase())
            .unwrap_or_default()
    } else {
        label
    };
    let desktop = string(&root["DesktopEntry"]);
    let mut did = if desktop.is_empty() {
        None
    } else if desktop.ends_with(".desktop") {
        Some(desktop.clone())
    } else {
        Some(format!("{desktop}.desktop"))
    };
    let mut info = did.as_ref().and_then(|d| gio::DesktopAppInfo::new(d));
    if info.is_none() {
        for app in gio::AppInfo::all() {
            let id = app.id().map(|s| s.to_string()).unwrap_or_default();
            let base = id.trim_end_matches(".desktop");
            if base.eq_ignore_ascii_case(short) || base.eq_ignore_ascii_case(&desktop) {
                if did.is_none() {
                    did = Some(id);
                }
                info = app.downcast::<gio::DesktopAppInfo>().ok();
                break;
            }
        }
    }
    let icon = info
        .and_then(|a| a.icon())
        .and_then(|i| gio::prelude::IconExt::to_string(&i))
        .map(|s| s.to_string())
        .unwrap_or_else(|| "multimedia-player".into());
    let ctl = p["CanControl"].as_bool().unwrap_or(true);
    let seconds = |v: &Value, empty_zero: bool| {
        pyint(v)
            .filter(|n| !empty_zero || *n != 0)
            .map(|n| round(n as f64 / 1_000_000., 3))
    };
    json!({"id":name,"identity":identity,"status":status,"title":string(&md["xesam:title"]),"artist":string(&md["xesam:artist"]),"album":string(&md["xesam:album"]),"artUrl":if art.is_empty(){None}else{Some(art)},"artPath":art_path,"canNext":truth(&p["CanGoNext"])&&ctl,"canPrevious":truth(&p["CanGoPrevious"])&&ctl,"canPlay":truth(&p["CanPlay"])&&ctl,"canPause":truth(&p["CanPause"])&&ctl,"canSeek":truth(&p["CanSeek"])&&ctl,"canControl":ctl,"length":seconds(&md["mpris:length"],true),"position":seconds(&p["Position"],false),"rate":p["Rate"].as_f64().filter(|n|*n!=0.).unwrap_or(1.),"desktopId":did,"icon":icon,"updated":now_ms()})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_and_capabilities() {
        let p = player_dict(
            "org.mpris.MediaPlayer2.test.instance",
            &json!({"Identity":"Music","DesktopEntry":"no-such-app"}),
            &json!({"PlaybackStatus":"Playing","Metadata":{"xesam:artist":["A","B"],"mpris:length":1234567},"Position":0,"CanControl":false,"CanGoNext":true}),
        );
        assert_eq!(p["artist"], "A, B");
        assert_eq!(p["length"], 1.235);
        assert_eq!(p["position"], 0.);
        assert_eq!(p["canNext"], false);
        assert_eq!(p["desktopId"], "no-such-app.desktop");
    }
    #[test]
    fn malformed_status_and_zero_length() {
        let p = player_dict(
            "org.mpris.MediaPlayer2.TEST",
            &json!({}),
            &json!({"PlaybackStatus":"unknown","Metadata":{"mpris:length":0}}),
        );
        assert_eq!(p["identity"], "Test");
        assert_eq!(p["status"], "stopped");
        assert!(p["length"].is_null());
    }
    #[test]
    fn control_errors_render_in_the_context_language() {
        // R2-A: the media stem in English through media.control (both errors come before any D-Bus call)
        for (language, control, player) in [
            ("en", "Unknown media control.", "Unknown player."),
            ("nl", "Onbekende bediening.", "Onbekende speler."),
        ] {
            let lc = crate::services::system::common::LangContext::new(language, "panel");
            // the struct itself, without new()'s MPRIS monitor thread
            let media = Media {
                ctx: lc.ctx.clone(),
                stopped: AtomicBool::new(true),
                last: Mutex::new(Value::Null),
                dirty: Mutex::new(None),
            };
            let e = media.control(&json!({"action":"rewind"})).unwrap_err();
            assert_eq!(
                e.json(),
                json!({"code":"bad_args","message":control,"key":"media.unknownControl","params":{}})
            );
            let e = media
                .control(&json!({"action":"play","id":"not.mpris"}))
                .unwrap_err();
            assert_eq!(
                (e.message.as_str(), e.key.as_deref()),
                (player, Some("media.unknownPlayer"))
            );
        }
    }
}
