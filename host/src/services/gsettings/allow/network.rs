//! Batch C (network and sharing): the allow table of `gsettings.*` (notes/claude/settings-parity.md §6.1). Only batch
//! C edits this file. The proxy page writes several schemas in one `gsettings.set {writes}` call, `mode` last
//! (amendment 51); authentication-password is never written.
//!
//! Write rows hold only the keys a page binds (§6.1: a key a source names only to say there is no control for it is
//! in no writing mode). GNOME Settings 50 has no control for proxy `use-same-proxy`, http `enabled`,
//! `use-authentication` and `authentication-user` (gnome-b.json: "have no UI"; the proxy page writes Windows' one
//! server to HTTP, HTTPS and FTP itself) nor for rdp `screen-share-mode` (gnome-a.json: "has no UI"): Read.
use super::Allow;
use super::Mode::{Read, Service, Write};

pub(super) const TABLE: &[Allow] = &[
    // Netwerk en internet › Proxy
    Allow::new(
        "org.gnome.system.proxy",
        &["mode", "autoconfig-url", "ignore-hosts"],
        Write,
    ),
    Allow::new("org.gnome.system.proxy", &["use-same-proxy"], Read),
    Allow::new("org.gnome.system.proxy.http", &["host", "port"], Write),
    Allow::new(
        "org.gnome.system.proxy.http",
        &["enabled", "use-authentication", "authentication-user"],
        Read,
    ),
    Allow::new("org.gnome.system.proxy.https", &["host", "port"], Write),
    Allow::new("org.gnome.system.proxy.ftp", &["host", "port"], Write),
    Allow::new("org.gnome.system.proxy.socks", &["host", "port"], Write),
    // Systeem › Extern bureaublad (the unit, TLS and port belong to sharing.rdp.set)
    Allow::new(
        "org.gnome.desktop.remote-desktop.rdp",
        &["enable", "view-only"],
        Write,
    )
    .optional("remote-desktop"),
    Allow::new(
        "org.gnome.desktop.remote-desktop.rdp",
        &["screen-share-mode"],
        Read,
    )
    .optional("remote-desktop"),
    Allow::new(
        "org.gnome.desktop.remote-desktop.rdp",
        &["port", "negotiate-port", "tls-cert", "tls-key"],
        Service("sharing.rdp.set"),
    )
    .optional("remote-desktop"),
    // Geavanceerde instellingen voor delen › Mediastreaming (gsd Sharing's own networks list)
    Allow::at(
        "org.gnome.settings-daemon.plugins.sharing.service",
        "/org/gnome/settings-daemon/plugins/sharing/{service}/",
        &["enabled-connections"],
        Service("sharing.media.set"),
    ),
];
