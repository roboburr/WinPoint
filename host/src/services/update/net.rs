//! NetworkManager's `Connectivity` and `Metered` (design §4.3 `network`, §4.6): NO_AUTO_START reads and
//! `PropertiesChanged` on the pk thread's connection. Only the owner calls it; readers use the snapshot's copy.
use gio::prelude::*;
use serde_json::{Value, json};

const NM: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

/// NM connectivity (`none|portal|limited|full|unknown`), online and metered (design §4.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Net {
    pub state: &'static str,
    pub online: bool,
    pub metered: bool,
}
impl Default for Net {
    /// No NetworkManager: nothing is known, so nothing is refused (online, not metered).
    fn default() -> Self {
        Self::from_nm(None, None)
    }
}
impl Net {
    /// `Connectivity` 0 unknown, 1 none, 2 portal, 3 limited, 4 full; `Metered` 1 yes, 3 guess-yes are metered.
    /// Online only for full and unknown (connectivity checking off), so a portal or a limited link is offline.
    pub(crate) fn from_nm(connectivity: Option<u32>, metered: Option<u32>) -> Self {
        let state = match connectivity {
            Some(1) => "none",
            Some(2) => "portal",
            Some(3) => "limited",
            Some(4) => "full",
            _ => "unknown",
        };
        Self {
            state,
            online: matches!(state, "full" | "unknown"),
            metered: matches!(metered, Some(1 | 3)),
        }
    }
    /// `update.check` refuses only a link NM reports as connectivity none (design §3.7 `update.err.offline`).
    pub(crate) fn refuses_check(&self) -> bool {
        self.state == "none"
    }
    pub(crate) fn json(&self) -> Value {
        json!({"state":self.state,"online":self.online,"metered":self.metered})
    }
    pub(crate) fn from_json(v: &Value) -> Self {
        let state = match v["state"].as_str() {
            Some("none") => "none",
            Some("portal") => "portal",
            Some("limited") => "limited",
            Some("full") => "full",
            _ => "unknown",
        };
        Self { state, online: v["online"] != false, metered: v["metered"] == true }
    }
}

fn get_u32(conn: &gio::DBusConnection, name: &str) -> Option<u32> {
    let reply = conn
        .call_sync(
            Some(NM),
            NM_PATH,
            PROPERTIES,
            "Get",
            Some(&(NM, name).to_variant()),
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            2000,
            gio::Cancellable::NONE,
        )
        .ok()?;
    super::pk::unboxed(reply.child_value(0)).get::<u32>()
}
/// One NO_AUTO_START read of both properties (an absent NetworkManager gives the default).
pub(crate) fn read(conn: &gio::DBusConnection) -> Net {
    Net::from_nm(get_u32(conn, "Connectivity"), get_u32(conn, "Metered"))
}
/// Subscribe to NM's `PropertiesChanged`; `changed` runs on the pk thread when `Connectivity` or `Metered`
/// is among the changed properties.
pub(crate) fn subscribe(conn: &gio::DBusConnection, changed: impl Fn() + 'static) -> gio::SignalSubscriptionId {
    conn.signal_subscribe(
        Some(NM),
        Some(PROPERTIES),
        Some("PropertiesChanged"),
        Some(NM_PATH),
        Some(NM),
        gio::DBusSignalFlags::NONE,
        move |_, _, _, _, _, params| {
            let props = params.child_value(1).get::<std::collections::HashMap<String, glib::Variant>>().unwrap_or_default();
            if ["Connectivity", "Metered"].iter().any(|key| props.contains_key(*key)) {
                changed();
            }
        },
    )
}
