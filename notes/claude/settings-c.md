# Settings parity, batch C (Network & sharing): handoff

Contract: `notes/claude/settings-parity.md` (§4.4 files, §5.4 host API, §5.8 polkit). Worktree
`/run/media/robin/22TB/win11-gnome-wt/c`, branch `claude/settings-c`, based on af8eebdc. Batch C is built in three
slices; each appends its part here.

## Slice 1: network, wifi, wifi-network, wifi-known, ethernet, net_editor.rs

### Host (`host/src/services/net.rs` + `net/{settings_map,connections,wifi,tests}.rs`)

Methods (all listed in `host/src/api-methods/network.json`, `METHODS` in net.rs; `!` dangerous, 🔑 secret):

| method | what it does |
|--------|--------------|
| `net.connections {type?}` | ListConnections + GetSettings: a summary per profile (uuid, id, type, autoconnect, timestamp, metered, permissions/shared, SSID, key-mgmt, eap, Bluetooth type, VPN type, active, device, state); active first, then last used. `type` is an NM type or `wifi` / `ethernet`. |
| `net.connection.get {uuid}` | every key of GetSettings as JSON (`variant_json`: ay as numbers, address-data, route-data, unknown keys too), plus `summary`, `unsaved`, `filename`. |
| `net.connection.update! 🔑 {uuid, settings, reapply?}` | GetSettings, the page's changes merged in (`settings_map::merge`: a value typed by the key table or by NM's own value, null removes a key, a null group removes the group; deprecated twins `addresses`, `routes`, `dns`, `cloned-mac-address` dropped when their replacement is there, or NM would ignore the change), Update2 flag TO_DISK, interactive. A change that sets a secret first merges every stored secret of the profile's secret groups (GetSecrets, interactive): NetworkManager keeps stored secrets only for an update without any. `reapply` (default true): Device.Reapply on the active devices, a reconnect when the device cannot reapply. |
| `net.connection.add! 🔑 {settings}` | `settings_map::build` (every key typed), a uuid from /proc when none, AddConnection2 TO_DISK, interactive → `{uuid, path}`. |
| `net.connection.delete! {uuid}` | Settings.Connection Delete, interactive. |
| `net.connection.activate! {uuid, device?}` | ActivateConnection (device = interface name, else "/"), interactive. |
| `net.connection.deactivate! {uuid}` | DeactivateConnection of its active connection (`net.notActive` when there is none). |
| `net.devices {accessPoints?, scan?}` | every device but loopback: type, state (`unplugged` for a wired device without carrier), driver, driver/firmware version, vendor and model (udev's `ID_VENDOR_FROM_DATABASE` / `ID_MODEL_FROM_DATABASE` of `/run/udev/data/n<ifindex>`, as libnm), MACs, MTU, speed / bitrate, bands, IPv4/IPv6 addresses, gateway, DNS (NameserverData or the old Nameservers), the active connection, the profiles it can use (uuids), for Wi-Fi the access point (band, channel, security) and with `accessPoints` every one in range; `scan` asks RequestScan first. Top level: networking, Wi-Fi and WWAN (hardware) switches, connectivity, connectivity check enabled/available, NM version. |
| `net.device.setEnabled! {iface, on}` | off = Device.Disconnect, on = ActivateConnection("/", device, "/") (GNOME's wired switch), interactive. |
| `net.wifi.hidden! 🔑 {ssid, security, secrets?, autoconnect?, iface?}` | a full hidden profile (`wifi::hidden_settings`: security kinds open, owe, wep, wep-passphrase, leap, dynamic-wep, wpa-psk, sae, wpa-eap; `secrets` only the 802-11-wireless-security and 802-1x groups), AddAndActivateConnection2 with `persist: disk` on the named or first Wi-Fi device, interactive. |
| `net.wifi.secrets 🔑 {uuid}` | GetSecrets("802-11-wireless-security"), interactive → `{ssid, keyMgmt, key, hidden, qr}`; `qr` is gnome-control-center's `get_qr_string` (`WIFI:S:…;T:WPA|SAE|WEP|nopass;P:…;H:true;;`, `\ ; , : "` escaped), null for an enterprise network. |
| `net.radios.set! {wifi?, wwan?}` | Properties.Set WirelessEnabled / WwanEnabled, interactive. |
| `net.connectivity.set! {on}` | Properties.Set ConnectivityCheckEnabled (polkit network-control), interactive. |

Every dangerous method checks its arguments under `ctx.dry_run` and touches no bus (the update's dry run merges the
change into a probe, so a wrong type is still `net.badValue`). Events: `net.changed {what}` after every write
(`connections`, `active`, `devices`, `radios`, `connectivity`), and a watcher thread (own MainContext) that turns
NetworkManager's signals into `net.changed {what}` (by object path: connections, active, devices, accessPoints at
most every 5 s, state) while a page listens. Errors: stem `net` (`host/i18n/net.tsv`, records in
`tests/i18n/BASELINE-CHANGES.rust-settings-c`): unavailable, badArgs, badValue (data group + key, never a value),
notFound, noDevice, noWifi, notActive, notWifi; daemon refusals are call_interactive's `core.*`.

Amendment 57: `system.network.setAirplane` (`host/src/services/system/network.rs`) sets gsd's
`org.gnome.SettingsDaemon.Rfkill` AirplaneMode on the session bus when gsd runs (GNOME Settings' own switch: Wi-Fi,
Bluetooth and WWAN together), and keeps nmcli/rfkill as the fallback; the airplane state reads AirplaneMode ||
HardwareAirplaneMode then. `system.network.connect` takes an optional `iface` (the Wi-Fi page's adapter when there are
several). No row in `allow/network.rs` was needed: these pages write no GSettings key.

### Native

* `pages/net_editor.rs` (no page): the shared dialogs, each reading the profile of `net.connection.get` and sending
  only its own keys: "IP-instellingen bewerken" (per family GNOME's methods with Windows' "Automatisch (DHCP)" and
  "Handmatig" first, address rows with subnet mask (IPv4) or prefix length (IPv6), gateway, and under
  "Geavanceerd" the routes, "Routes automatisch ophalen" and "Deze verbinding alleen gebruiken voor bronnen in het
  eigen netwerk"), "DNS-serverinstellingen bewerken" (Automatisch (DHCP) / Handmatig with Voorkeurs-DNS,
  Alternatieve DNS and more per family, plus GNOME's "Ook de DNS-servers van het netwerk gebruiken"), the 802.1X form
  (PEAP, TTLS, TLS, PWD, FAST, LEAP with GNOME's fields, inner methods, PEAP version, PAC provisioning, certificates
  through a file dialog, "Wachtwoord opslaan" = the secret flags 1 / 0 / 2), "Verificatie-instellingen bewerken"
  (Ethernet), the Wi-Fi security form (GNOME's nine types, WEP index and authentication, LEAP, the key; an empty key
  keeps the stored one), the key view with the QR code (`pickers::QrCode`), the new-network dialog and the Windows
  property list (`properties`) with Kopiëren.
* `pages/network.rs`: **network** keeps the status card, Wi-Fi and Ethernet rows; adds GNOME's "Netwerk niet
  beschikbaar" warning when NetworkManager does not run, and Windows' rows in Windows' order: Mobiel netwerk (caps
  modem), VPN, Mobiele hotspot (its switch appears once `net.hotspot.get` answers, slice 2), Vliegtuigmodus (link
  and switch), Proxy, Geavanceerde netwerkinstellingen; Eigenschappen opens the active Wi-Fi's properties.
  **wifi** keeps the switch and the list; adds per saved network the Eigenschappen button (GNOME's gear), the
  "Verborgen netwerk" entry (`net.wifi.hidden`), GNOME's adapter switcher when there are several adapters (that list
  comes from `net.devices` access points, connect/disconnect pass the adapter), "Eigenschappen van <SSID>" for the
  connected network, Bekende netwerken beheren, Hardware-eigenschappen (with Kopiëren); follows `net.changed`.
  **ethernet** per adapter: the switch with GNOME's status ("Verbonden, 1000 Mbps", "Netwerkkabel losgekoppeld"),
  Verificatie-instellingen, Verbinding met een datalimiet, IP-toewijzing, DNS-servertoewijzing, Geavanceerd (dialog:
  Naam, MAC-adres, Gekloond adres, MTU), Eigenschappen with Kopiëren, Profielen (activate, add, remove), and GNOME's
  Bluetooth network (PAN/DUN: switch, Naam wijzigen); rebuilt when the adapters' state changes; without `net.*` the
  old sysmon page.
* `pages/wifi_network.rs`: **wifi-network** (data page `{uuid}`, title = the SSID): Automatisch verbinding maken als
  ik binnen bereik ben, Beschikbaar maken voor andere gebruikers (`permissions` [] or `user:<name>:`), Willekeurige
  hardwareadressen (Uit/Aan/Dagelijks wijzigen = permanent/stable/random, plus the profile's own value), metered, IP,
  DNS, Beveiligingstype · Bewerken, Wi-Fi-beveiligingssleutel weergeven (with QR), Eigenschappen with Kopiëren and
  Laatst gebruikt, Geavanceerd (BSSID from seen-bssids, the adapter's MAC), Dit netwerk vergeten; follows
  `net.changed`. **wifi-known**: Een nieuw netwerk toevoegen (name, type, key, Automatisch verbinding maken,
  Verbinding maken, zelfs als dit netwerk niet uitzendt → `net.connection.add`), the list with "Zoeken in deze lijst",
  Vergeten with Windows' Ongedaan maken (the profile goes after the toast, also when the page is left).

Bound controls are named `st-<page>-<NM key>` (`st-wifi-network-autoconnect`, `-permissions`,
`-assigned-mac-address`, `-bssid`, `-mac-address`, `st-<page>-metered`, `st-ethernet-<iface>-enabled`); the dialogs'
fields `net-ip-<family>-…`, `net-dns-…`, `net-eap-…`, `net-security-…`, `net-new-…`.

### plan.json controls dropped or changed (with the reason)

* Amendment 40, the libnma spike: libnma-gtk4 1.10 here exports `nma_ws_*` (since 1.8.28) and
  `nma_wifi_dialog_new_for_hidden`, but (a) libnma has no Dutch catalogue on this system (only the en_GB/en_CA
  langpacks), so its forms would be English in a Dutch WinPoint, (b) its widgets are GNOME's grids with libnma's
  wording, not Windows' cards and texts, (c) they need an NMConnection in the Settings process (libnm through dlopen
  and the secrets there). Outcome: WinPoint's own forms in Windows' wording with the keys GNOME's forms write. The
  spike was a static one (exports and catalogues checked; no embedding was tried). Gap: libnma's rarer 802.1X options
  (subject match, alternative subject matches, the system CA folder, phase-2 certificates, the "no CA certificate is
  required" checkbox) have no field; the merge keeps their values.
* IP dialog: Windows has one "Automatisch (DHCP) / Handmatig" choice with IPv4 / IPv6 switches; GNOME has five (IPv4)
  and seven (IPv6) methods. One drop-down per family holds all of them, Windows' two first, so nothing is lost;
  GNOME's routes and never-default sit under "Geavanceerd" (windows.json's advice).
* Windows' "DNS via HTTPS", "Type netwerkprofiel", "Een datalimiet instellen" and the Wi-Fi page's global
  "Willekeurige hardwareadressen": no GNOME backing (DoH is not NM's DoT; no firewall zones; no data limits; the
  global random MAC is NetworkManager.conf only): left out.
* Property list "Protocol" (Wi-Fi 4/5/6): NetworkManager reports no standard; left out rather than guessed. Link
  speed shows NM's one bit rate as receive/transmit.
* Ethernet "Gekloond adres": GNOME also lets you type an explicit MAC; the drop-down keeps an existing explicit MAC
  but offers no field to type a new one.
* The network page's hotspot switch needs `net.hotspot.*` (slice 2): it reads `{active, ssid, password, band}` from
  `net.hotspot.get` and calls `net.hotspot.start {ssid, password, band}` / `net.hotspot.stop {}`; slice 2 answers in
  that shape or adjusts `network.rs` (`network()`, the hotspot card).

### Notes for I

* None of slice 1's work needs a frozen file. `common::placeholder` is no longer used by network, wifi, ethernet,
  wifi-network or wifi-known.
* Not batch C's, seen in every batch's fresh build: `services::update::elf::tests::our_binaries_equal_readelf` fails
  once `tests/build/native-release` holds binaries built from af8eebdc, because the host's binaries (win11-app,
  win11-default-apps, win11-explorer, win11-file-portal, win11-installer, win11-panel) now also NEED `libm.so.6`
  (the a, b, d, e and f worktrees' fresh win11-app show the same). The recording
  `tests/update/fixtures/compat/readelf-needed.txt` needs `tests/update/test_compat.sh --record-readelf` after the
  merge (the test passed here while native-release still held the older binaries).
* `runtime::is_secret` exists, nothing calls it yet: net.connection.update / add, net.wifi.hidden and
  net.wifi.secrets are `"secret": true`; the harness redaction is I's (amendment 53).

### Tests (slice 1)

* Host unit tests (`wt/host-tests.sh c --lib`): 589 run, 588 passed, 1 failed (the readelf recording above, not
  batch C's), 25 ignored. Batch C's: 15 in `services::net::tests` (the settings map on a GetSettings fixture with
  keys and groups no row knows: merge, null, legacy twins, round trip; bad values with their group and key; value
  kinds; secrets; the WIFI: QR text; hidden profiles; every security kind; bands, channels, states, AP security;
  udev names; summaries; signal kinds and argument checks; every dangerous method's dry run touches no bus while the
  real call needs NetworkManager; the D-Bus argument types; the net texts in both languages), plus the 3 of
  system/network.rs.
* D-Bus fakes (`tests/dbus-fakes/run.sh --batch c --strace -- <lib test binary> --ignored --exact …`, one fresh fake
  per test, fakes/c/networkmanager.py, which refuses any in-signature NetworkManager would refuse): 6 of 6 passed,
  strace OK each time: the update's Update2 keeps zone, seen-bssids, proxy and the unknown group, drops `addresses`,
  saves to disk, allows the password dialog, then Reapply; a psk change asks GetSecrets first; the key and QR
  text; devices and connections; add, activate, deactivate, device off/on, radios, connectivity, hidden network and
  delete, each with ALLOW_INTERACTIVE_AUTHORIZATION; flight mode through gsd's Rfkill on the session bus.
  The first run caught a real bug (the settings dictionary was sent wrapped in a variant), fixed in 98d09af8.
* Native (`wt/native-tests.sh c`): every suite green; win11-gtk-settings 33 passed, 9 of them batch C's
  (net_editor 6, network 2, wifi_network 1). No new warnings (host lib 28, win11-shell 19, as the base).
* `wt/checks.sh c`: as the base (rustfmt 0 failing, check_rust 0 errors / 57 warnings, drift 0 errors / 15
  warnings, native 0 errors, test_native OK, settings-pages OK, release OK; the installer check refuses to run in
  this environment, XDG_RUNTIME_DIR not 0700, before and after).
* Harness (`tests/gnome/scenarios/settings-network.py`, the fake with `--system-only` on the private system bus):
  nl 25 checks, en 30 checks, 0 failed: every page with its title, no placeholder, no Dutch word in en; a real
  pointer click on "Automatisch verbinding maken als ik binnen bereik ben" sent Update2 with autoconnect false, the
  profile's other keys kept and the interactive flag. Screenshots `<page>.<lang>.png` in settings-network.out and
  settings-network.en.out (PNGs not committed).

### Open (slice 1)

* Not run against a real NetworkManager (only the fakes); the polkit prompts themselves are GNOME Shell's.
* The harness scenario covers one write (autoconnect); the dialogs (IP, DNS, security, 802.1X, new network) are
  covered by their pure helpers and the host's merge tests, not by clicks.

## Slice 2: mobile, vpn, hotspot, flightmode, proxy

### Host (`host/src/services/net.rs` + `net/{hotspot,vpn,modem,providers}.rs`, `settings_map.rs` additions)

Methods (14 new, all in `host/src/api-methods/network.json` and `METHODS`; `!` dangerous, 🔑 secret):

| method | what it does |
|--------|--------------|
| `net.hotspot.get 🔑 {secrets?}` | GNOME's hotspot (`is_hotspot_connection`: Wi-Fi in mode ap or adhoc with IPv4 shared): available (a Wi-Fi adapter), iface, apCapable, bands, active, state, configured, uuid, ssid, band, keyMgmt, security. The stored key (and the WIFI: QR text) only with `secrets: true` (GetSecrets, interactive), so the overview page and the hotspot page never prompt on load. Without a stored hotspot: the computer's name (hostname1 PrettyHostname, else the host name, at most 32 bytes) and a new random key, what turning it on uses. Plus `uplink` (the primary connection: Windows' "Mijn internetverbinding delen vanaf") and `sameAdapter` (the uplink runs on the hotspot's adapter, so turning it on drops that connection). |
| `net.hotspot.start! 🔑 {ssid?, password?, band?, iface?, activate?}` | An existing hotspot profile changed in place (Update2 merge; null or absent keeps a value) and activated again (GNOME's commit then activate); else GNOME's new profile ("Hotspot", autoconnect false, mode ap or adhoc, IPv4 shared, IPv6 ignore, WPA2/CCMP, WPA/TKIP or a WEP key by the adapter's capabilities) through AddAndActivateConnection2 (persist disk). `band` is Windows' (a, bg, or "" for any); `activate: false` only saves (Windows' Bewerken while the hotspot is off; AddConnection2 for a new one). The key is checked against the profile's key management (WPA 8-63 / 64 hex, WEP 5/13 / 10/26). |
| `net.hotspot.stop! {}` | DeactivateConnection of every active hotspot; `{stopped: 0}` when none runs. |
| `net.vpn.plugins {}` | The `.name` files of /usr/lib/NetworkManager/VPN and /etc/NetworkManager/VPN (libnm's order; a service once): name, service, program, editor, import; `wireguard: true`. |
| `net.vpn.import! {path, type?}` | A regular file of at most 1 MiB, its kind given or detected (WireGuard `[Interface]` + PrivateKey, OpenVPN `.ovpn` or its directives, Cisco `.pcf`), a WireGuard file named after a valid interface (NetworkManager's rule); the plugin must be installed; then `nmcli connection import type <kind> file <path>` (`tools::system_tool`, LC_ALL=C, 60 s) and the uuid from nmcli's answer. The dry run reads and checks the file and starts nothing. |
| `net.modem.get {}` | ModemManager's GetManagedObjects (UnlockRetries `a{uu}` read by hand): per modem manufacturer, model, firmware, IMEI, numbers, state, failedReason, unlockRequired, unlockRetries, technologies, networkType (5G/4G/3G/2G), signal, supported/current modes with names, SIM (iccid, imsi, operator, mcc, mnc, eSIM), SIM slots, primary slot, registration, operator, pinEnabled (facility lock SIM); NetworkManager's modem device (Udi = the modem's path: iface, state) and the gsm profiles of its SIM (by AvailableConnections, else gsm.sim-id / device-id): uuid, id, apn, username, autoconnect, homeOnly, attach, auth (from the ppp refuse flags), active, state. `available: false` without ModemManager. |
| `net.modem.unlock! 🔑 {modem, pin}` / `{modem, puk, newPin}` | Sim.SendPin / Sim.SendPuk, interactive. |
| `net.modem.setPin! 🔑 {modem, enabled, pin}`, `net.modem.changePin! 🔑 {modem, old, new}` | Sim.EnablePin / Sim.ChangePin, interactive. |
| `net.modem.setModes! {modem, allowed, preferred}` | One of the modem's SupportedModes (checked against them), Modem.SetCurrentModes((uu)), interactive. |
| `net.modem.scan {modem}` | Modem3gpp.Scan (interactive, up to call_interactive's timeout): `{networks: [{code, name, short, status, networkType}]}`, the current one first, forbidden ones last. |
| `net.modem.register! {modem, operator?}` | Modem3gpp.Register ("" = automatic; else 5-6 digits), interactive. |
| `net.modem.setPrimarySlot! {modem, slot}` | A slot the modem has (1-based), Modem.SetPrimarySimSlot, interactive. |
| `net.modem.apns {mcc, mnc}` | mobile-broadband-provider-info's serviceproviders.xml (roxmltree, its external DTD allowed and never loaded): providers [{name, country, apns: [{apn, name, usage, username, password, auth, dns}]}]. |

`settings_map.rs`: the kinds of the WireGuard (`private-key`, `listen-port`, `fwmark`, `mtu`, `peer-routes`, `ip4/ip6-auto-default-route`, `peers` as a new `Peers` kind `aa{sv}` that needs a public key per peer), gsm (apn, username, password, pin, sim-id, sim-operator-id, device-id, network-id, number, home-only, auto-config, initial-eps-bearer-configure/-apn, the flags, mtu) and ppp keys (refuse-*, noauth, MPPE, compression, lcp-echo, mtu/mru), `vpn.persistent`/`timeout`. Secrets: `vpn.secrets` in a change sets or removes single entries over the stored ones (`add_secrets` now merges the GetSecrets dictionary into it), and a WireGuard peer keeps the pre-shared key of its public key when the page did not type it again; a peer with a pre-shared key makes the change a secret one (so GetSecrets runs first). The watcher also follows ModemManager's signals (`net.changed {what: "modem"}`, at most every 5 s, like the access points).

`system.network.get` (amendment 57's file) gained `airplaneHardware` (gsd's HardwareAirplaneMode, else every rfkill radio hard-blocked): the flight mode page's notice.

Errors (stem `net`, rows in `host/i18n/net.tsv`, records in `tests/i18n/BASELINE-CHANGES.rust-settings-c`): badPassword, badSsid, badImport, badImportName, importFailed (data error: nmcli's English message), noPlugin, noModem, noSim, badPin. No new allow row: the proxy page writes only B0's `allow/network.rs` rows (a host test sends the page's writes through `gsettings::write_values` and checks that `authentication-password` is refused).

Contract additions (section 5.4 allows optional arguments): `net.hotspot.get` takes `secrets`; `net.hotspot.start`'s ssid/password/band are optional (keep / GNOME's default) and it takes `activate`; `net.vpn.import` takes `type`. Slice 1's overview switch sends what `net.hotspot.get` answers (a null password keeps the stored key), and now hides itself without a Wi-Fi adapter.

### Native

* `pages/proxy.rs` **proxy**: "Automatische proxyinstellingen" (Instellingen automatisch detecteren: a switch = mode auto with an empty autoconfig-url, GNOME's WPAD warning under it while it is on; Installatiescript gebruiken · Instellen: the dialog's switch and Scriptadres = mode auto with the URL) and "Handmatige proxyinstellingen" (Een proxyserver gebruiken · Instellen: the switch, IP-adres van proxy and Poort, Windows' exceptions text with `;`, De proxyserver niet gebruiken voor lokale (intranet)adressen, and under Geavanceerd "Dezelfde proxyserver voor alle protocollen gebruiken" plus GNOME's HTTPS, FTP and SOCKS servers). Every Opslaan is one `gsettings.set {writes}`, the servers and ignore-hosts first, `mode` last. The cards say what is on (the URL, host:port, Uit); a locked mode makes the controls insensitive with the lockdown tooltip; follows `gsettings.changed`. Related: Geavanceerde netwerkinstellingen.
* `pages/flightmode.rs` **flightmode**: Vliegtuigmodus (system.network.setAirplane; insensitive while the hardware switch holds it), GNOME's "Vliegtuigmodus via hardwareschakelaar staat aan" notice, Draadloze apparaten: Wi-Fi (system.network.setWifi), Bluetooth (system.bluetooth.setPowered, batch B's), Mobiel netwerk (net.radios.set wwan, only with caps.modem); follows system.network.changed, system.bluetooth.changed and net.changed radios. Related: Wi-Fi, Bluetooth.
* `pages/hotspot.rs` **hotspot**: the switch (on: net.hotspot.start with the shown properties, first GNOME's warning when the uplink runs on the same adapter; off: GNOME's "Turn Off Hotspot?" question, then net.hotspot.stop), Mijn internetverbinding delen vanaf (the uplink), Delen via (Wi-Fi), Eigenschappen (Naam, Wachtwoord, Band, Beveiligingstype; "Wachtwoord weergeven" asks net.hotspot.get with secrets and then shows the key and GNOME's QR code), Bewerken: "Netwerkinformatie bewerken" (Netwerknaam, Netwerkwachtwoord with GNOME's Willekeurig wachtwoord genereren, Netwerkband with the adapter's bands; saved with activate false while the hotspot is off); the no-Wi-Fi notice; follows net.changed.
* `pages/vpn.rs` **vpn**: "VPN-verbindingen" with VPN toevoegen and GNOME's Importeren uit bestand… (a file dialog for .ovpn/.conf/.pcf, then net.vpn.import); one expander per VPN or WireGuard profile with the status and provider, Verbinding maken / Verbinding verbreken, Geavanceerde opties · Bewerken, IP-instellingen · Bewerken (net_editor's IPv4/IPv6 dialog), Beschikbaar maken voor andere gebruikers, Verwijderen (with the question). "Een VPN-verbinding toevoegen": VPN-provider (the installed OpenVPN and PPTP plugins, WireGuard), Verbindingsnaam, Servernaam of -adres, for OpenVPN Type aanmeldingsgegevens (password, TLS, password with TLS, static key) with the CA certificate, user certificate, private key and its password, static key with local and remote address, Gebruikersnaam (optioneel), Wachtwoord (optioneel), Mijn aanmeldingsgegevens onthouden (password-flags 1, the user's keyring through GNOME Shell's agent as GNOME's default; off 2, asked every time); for WireGuard the interface name, private key, this computer's tunnel address, the server's public key, allowed IPs (0.0.0.0/0, ::/0) and an optional pre-shared key. Geavanceerde opties: OpenVPN's common keys (remote, port, TCP, user name, CA, certificate, key, static key and direction, local/remote IP, device type, cipher, HMAC, TLS auth key and direction, tls-crypt, remote-cert-tls, tunnel MTU) or PPTP's (gateway, user, NT domain, MPPE, 128-bit, stateful, the "toestaan" boxes of refuse-*, compression), the password (empty keeps it) and every other vpn.data key as a name/value row; WireGuard: name, interface, private key (empty keeps it), listen port, fwmark, MTU, peer routes and the peers (public key, endpoint, allowed IPs, keepalive, pre-shared key). Follows net.changed. Related: Proxy, Geavanceerde netwerkinstellingen.
* `pages/mobile.rs` **mobile**: GNOME's status pages (Vliegtuigmodus staat aan with "Vliegtuigmodus uitschakelen", no mobile broadband adapter), the Mobiel netwerk switch (net.radios.set wwan) with the modem's status ("Verbonden met KPN, 4G"), a Modem switcher and "Deze simkaart gebruiken voor mobiele gegevens" with several modems, the SIM notices (locked with its retries and Ontgrendelen: the PIN, or the PUK with a new PIN twice; no SIM; SIM error), Mobiele gegevens (the gsm profile's autoconnect plus activate/deactivate on the modem's interface; without a profile the operator's first internet access point becomes one, else the APN dialog opens), Roaming (Niet roamen / Roamen = gsm.home-only), Primaire simsleuf (with the question: the modem restarts), "Instellingen van mobiele provider": Voorkeursnetwerktype (the supported mode pairs, "4G, 3G, 2G (voorkeur: 4G)"), Netwerk selecteren · Kiezen (the scan, Automatisch, Register), Toegangspunt (the SIM's access points: Toepassen, Bewerken, Verwijderen; APN toevoegen / APN bewerken with Profielnaam, APN, Gebruikersnaam, Wachtwoord, Type aanmeldingsgegevens (Automatisch, Geen, PAP, CHAP, MS-CHAP v2 = the ppp refuse flags), APN-type (Internet, Internet en koppelen) and GNOME's presets from serviceproviders.xml), Beveiliging van simkaart (Pincode van simkaart gebruiken, Pincode van simkaart wijzigen), Eigenschappen with Kopiëren (provider, network type, signal, status, number, manufacturer, model, firmware, IMEI, ICCID). Follows net.changed, rebuilt only when what it shows changed (not for the signal). Related: Vliegtuigmodus.
* `pages/net_editor.rs`: its form helpers (Rows, file_field, password_entry, section, show_characters, options, tr) are `pub(super)` for these pages. `pages/network.rs`: the hotspot switch only with a Wi-Fi adapter.

Control names: `st-proxy-mode` (the auto-detect switch), `st-proxy-autoconfig-url`, `st-proxy-ignore-hosts`, `st-proxy-<http|https|ftp|socks>-<host|port>` (gs::control_name), `proxy-script-edit`, `proxy-manual-edit`, `proxy-*-on`, `proxy-local`, `proxy-same`; `flightmode-<airplane|wifi|bluetooth|mobile>`, `flightmode-hardware`; `hotspot-switch`, `hotspot-edit`, `hotspot-show`, `hotspot-edit-*`; `vpn-add`, `vpn-import`, `vpn-<n>` (+ `-connect`, `-advanced`, `-ip`, `-remove`), `st-vpn-<n>-permissions`, `vpn-new-*`, `vpn-adv-*`, `vpn-wg-*`; `mobile-switch`, `mobile-sim`, `mobile-unlock`, `st-mobile-autoconnect`, `st-mobile-home-only`, `st-mobile-apn`, `mobile-*`.

### plan.json controls dropped or changed (slice 2)

* Amendment 40, the GTK 4 VPN editors: `libnm-gtk4-vpn-plugin-{openvpn,pptp}-editor.so` export only `nm_vpn_editor_factory_{openvpn,pptp}` and NEED libgtk-4, libnma-gtk4 and libnm; NetworkManager-openvpn and -pptp have no Dutch catalogue here (only en_GB in the langpacks), so the embedded editors would be English GNOME grids in a Dutch Windows layout, and they need an NMConnection (libnm) in the Settings process. Static spike again (exports, NEEDED, catalogues; no embedding tried). Outcome: WinPoint's forms for the common keys and the name/value rows the plan names for the rest (kept as NetworkManager has them); Add VPN offers OpenVPN, PPTP and WireGuard, other installed plugins (vpnc, l2tp, openconnect, ...) can be imported or edited through the name/value rows only.
* VPN: Windows' "VPN-type" is the provider choice (OpenVPN's connection types are under Type aanmeldingsgegevens); "Geavanceerde instellingen voor alle VPN-verbindingen" (metered, roaming) and the VPN proxy row: no GNOME backing, left out; WireGuard's add form takes one peer (more under Geavanceerde opties).
* VPN import through nmcli (the plan's choice): nmcli runs without a polkit agent of its own, so `settings.modify.system` must be YES (Ubuntu's rule for sudo and netdev users); otherwise nmcli's "insufficient privileges" comes back as net.importFailed. GNOME imports into its editor and saves through AddConnection; doing that here would need the plugin's importer in-process.
* Hotspot: Windows' "Delen via Bluetooth", "Energiebesparing" and "Apparaten verbonden: x van 8" have no GNOME or NetworkManager backing (no station list for an AP): left out. "Mijn internetverbinding delen vanaf" is information (NetworkManager's shared mode uses the default route). The band is Windows' (NetworkManager's 802-11-wireless.band), GNOME has none. The key shows after "Wachtwoord weergeven", not on load (GetSecrets may prompt).
* Flight mode: Windows' note that Wi-Fi and Bluetooth are remembered has no backing (gsd decides).
* Proxy: GNOME's mode is one value, Windows' three settings are switches: turning one on turns the others off (said on the page). "Lokale (intranet)adressen" is `*.local` in ignore-hosts (GNOME, glib and libproxy have no "names without a dot" rule). Windows' one server is written to HTTP, HTTPS and FTP while "Dezelfde proxyserver voor alle protocollen gebruiken" is on, not through `use-same-proxy` (libproxy's GNOME backend ignores that key); `use-same-proxy`, http `enabled`, `use-authentication` and `authentication-user` are not written (no control in GNOME either; Read rows since the review, contract §6.1), `authentication-password` never.
* Mobile: Windows' "Windows mag me verbonden houden" is the Mobiele gegevens switch (GNOME's Mobile Data = autoconnect + activate). APN "IP-type": no GNOME control, left out. APN-type Koppelen is "Internet en koppelen" (gsm.initial-eps-bearer-configure + -apn on the same profile; only those two attach keys are written, the attach user name and password keys are not sent because their names in NetworkManager 1.54 could not be checked here). gsd-wwan's `unlock-sim` key has no GNOME control: not offered.

### Notes for I (slice 2)

* No frozen file needed. `common::placeholder` is no longer used by mobile, vpn, hotspot, flightmode or proxy.
* `runtime::is_secret` (amendment 53): net.hotspot.get, net.hotspot.start, net.modem.unlock, net.modem.setPin and net.modem.changePin are `"secret": true` too.
* The D-Bus fakes changed for slice 2 (slice 1's tests still pass with them): `fakes/c/networkmanager.py` has an OpenVPN, a WireGuard and a gsm profile, a modem device (cdc-wdm0, Udi = the ModemManager modem), WwanEnabled true, an AP-capable wlan0, ActiveConnection objects for (de)activations, and keeps secrets out of GetSettings (GetSecrets answers them, an Update2 with secrets replaces them); `fakes/c/modemmanager.py` is new (a locked SIM, PIN 1234, PUK 12345678).

### Tests (slice 2)

* Host unit tests (`wt/host-tests.sh c --lib`, full run): 625 run, 597 passed, 1 failed (`services::update::elf::tests::our_binaries_equal_readelf`, the libm.so.6 recording of slice 1's note, not batch C's), 27 ignored. `services::net::tests`: 24 passed (9 new: GNOME's hotspot profile per adapter capability and its merge; hotspot keys, random keys, names, changes and arguments; VPN plugins from `.name` files; import file detection, WireGuard file names, nmcli's answers; WireGuard peers and vpn.secrets keeping what the page did not send, gsm/ppp types; a ModemManager GetManagedObjects fixture as the page's modem; APN sign-in flags and gsm profiles; serviceproviders.xml presets (a fixture and the installed file); the proxy page's writes through `gsettings::write_values`, the password refused), 7 ignored (the bus tests). The dry-run test now also covers net.hotspot.start/stop, net.vpn.import (reads and checks the file, starts no nmcli) and every net.modem write, with their refusals (badPin, badSsid, badPassword, badImport, badImportName, badArgs).
* D-Bus fakes (`tests/dbus-fakes/run.sh --batch c --strace -- <lib test binary> --ignored --exact …` inside tools/heavy.sh, one fresh set of fakes per test): 8 of 8 passed, strace OK each time: slice 1's six with the changed NetworkManager fake, and the two new ones. Hotspot: get proposes the computer's name, a key of 8 and WPA2 on the AP-capable wlan0 with Lab-5G as the uplink on the same adapter; start sends AddAndActivateConnection2 (Hotspot, autoconnect false, mode ap, band bg, IPv4 shared, IPv6 ignore, wpa-psk, rsn, ccmp, the key, Devices/1, persist disk, interactive); get with secrets reads the key and the QR text through an interactive GetSecrets; a change is Update2 of the same profile (the band removed, the rest kept, no GetSecrets) then ActivateConnection; activate false saves only; stop deactivates every active hotspot. Modem: get shows the locked SIM with 3 tries, cdc-wdm0 and the gsm profile of the SIM; a wrong PIN costs a try; SendPin('1234') on the SIM, interactive, unlocks it (home, KPN, 4G); SetCurrentModes, Register (also automatic), EnablePin, ChangePin and SetPrimarySimSlot with their exact arguments and the interactive flag; an unsupported mode pair, a missing slot and an unknown modem are refused before any call; the scan lists the current network first and a forbidden one last; the reads never allow a password dialog.
* Native (`wt/native-tests.sh c`): every suite green, 83 passed in total (1 ignored in the kit); win11-gtk-settings 48 passed, 15 new (proxy 4, vpn 4, mobile 4, hotspot 2, flightmode 1).
* Builds (`wt/build.sh c`): OK, no new warnings (host lib 28, win11-launch 1, win11-shell 19, win11-gtk-settings 0, as the base).
* `wt/checks.sh c`: as the base: rustfmt 0 failing (189 files, 159 clean), check_rust 0 errors / 57 warnings, drift 0 errors / 15 warnings (`tests/i18n/allow-host-settings-c.txt` names the ppp key `refuse-mschap` and `mschap`, which the Dutch guard took for Dutch), native 0 errors and 48 uncovered (the VPN key tables are documented const tables), test_native OK, settings-pages OK, release OK; the installer check refuses to run here (XDG_RUNTIME_DIR not 0700), before and after.
* Harness (`tests/gnome/scenarios/settings-network.py`, the NetworkManager fake `--system-only` and the ModemManager fake on the private system bus): nl 60 checks, en 70 checks, 0 failed. Slice 2: flight mode, proxy, VPN (both profiles, Add VPN, Import), hotspot and mobile open with their titles, no placeholder and no Dutch in en; a real pointer click on "Instellingen automatisch detecteren" writes mode auto with an empty autoconnect-url into the session's GSettings and shows GNOME's WPAD warning, a second click mode none; the hotspot switch shows the question (wlan0 carries Lab-5G), then AddAndActivateConnection2 of a hotspot profile with the interactive flag; the mobile page shows the locked SIM, its unlock dialog sends SendPin('1234') with the interactive flag and the page then shows Mobiele gegevens. The dialog buttons and the unlock button are pressed through the dev channel: in the first nl run the pointer clicks on them had no effect (cause not found), while the switches take real pointer clicks. Screenshots `<page>.<lang>.png`, `proxy-auto`, `hotspot-question`, `mobile-unlock`, `mobile-unlocked` (PNGs not committed).

### Open (slice 2)

* Not run against a real NetworkManager, ModemManager or polkit agent; no modem or second Wi-Fi radio here. The hotspot's AP mode on a real adapter, a real SIM's PUK path, the operator scan's duration and nmcli's import of real files are unverified.
* The VPN import uses nmcli without a password agent of its own: a user without the Ubuntu sudo/netdev rule gets nmcli's refusal instead of GNOME's password dialog.
* The plugins other than OpenVPN and PPTP (vpnc, l2tp, openconnect, strongswan, fortisslvpn, sstp) are offered for import and as name/value rows only, without their own forms; their secrets beyond `password` have no field.
* The attach-bearer user name and password of an APN (GNOME's Attach APN with sign-in) are not written; nor are Windows' APN "IP-type", the hotspot's "Delen via Bluetooth" and "Energiebesparing", and Windows' global VPN switches (no backing).
* The kit dialog's buttons and an info bar's action button did not react to the harness's pointer click in one run (the dev channel's click works); not investigated further (the harness and the kit are not batch C's).

## Slice 3: network-advanced, sharing-advanced, remotedesktop, system-advanced

### Host (`host/src/services/sharing.rs` + `sharing/{rdp,ssh,media,systemd,tls,tests}.rs`)

Methods (15, all in `host/src/api-methods/network.json` and `METHODS`; `!` dangerous, 🔑 secret):

| method | what it does |
|--------|--------------|
| `sharing.rdp.get {}` | Desktop Sharing: available (the `org.gnome.desktop.remote-desktop.rdp` schema), enabled, viewOnly, port (the running daemon's own from `org.gnome.RemoteDesktop.User` `/org/gnome/RemoteDesktop/Rdp/Server` Port, also a negotiated one, else the key), configuredPort, negotiatePort, running, unit (`gnome-remote-desktop.service` of the user manager: exists, fileState, active), hostname (the kernel's), certificate + fingerprint (SHA-256 of `tls-cert`'s DER, `AB:CD:…`), grdctl, tls (libgnutls loads). |
| `sharing.rdp.set! {on}` | As cc-desktop-sharing-page.c: on = the certificate (the configured pair when GIO loads it, else GNOME's pair in `~/.local/share/gnome-remote-desktop/certificates/rdp-tls.{crt,key}`, made with GnuTLS when missing; `tls-cert`/`tls-key` written through `gsettings::write_values("sharing.rdp.set", …)`), `enable` true, the credentials GNOME's page fills in when the keyring has none (the login name and a new 16-character password, `grdctl rdp set-credentials`), then the user unit StartUnit + EnableUnitFiles; off = `enable` false, StopUnit + DisableUnitFiles. Answers `{on, credentials: kept|stored|failed}` (a keyring that cannot be reached does not stop the switch: GNOME also only warns). |
| `sharing.rdp.credentials.get 🔑 {system?}` | `grdctl status --show-credentials` (LC_ALL=C; the `Username:`/`Password:` lines of its `RDP:` section, "(null)"/"(hidden)" = none) → `{username, password}`; with `system: true` the configuration daemon's GetCredentials (interactive). `system` is an optional argument this slice added (§5.4 allows it): the remote login page needs the stored details and the contract has no other getter. |
| `sharing.rdp.credentials.set! 🔑 {username, password}` | `grdctl rdp set-credentials <username>` with the password as one line on grdctl's standard input, never on a command line. Probed in the dbus-fakes layer with a private gnome-keyring: grdctl 50.2 segfaults when both are prompted for (no arguments) or when the standard input is empty, and takes the one-argument form; so the user name may not start with `-` and neither may hold a control character. |
| `sharing.rdp.fingerprint {}` | `{fingerprint}` of Desktop Sharing's certificate (null without one). |
| `sharing.rdp.system.get {}` | Remote Login: the system daemon `org.gnome.RemoteDesktop.Configuration` (`/org/gnome/RemoteDesktop/Configuration`, `.Rdp.Server`, D-Bus activated as the gnome-remote-desktop user): available, enabled, port, certificate, fingerprint (its TlsFingerprint in the page's form), hostname. |
| `sharing.rdp.system.set! {on}` | on = when the daemon has no certificate, GNOME's new pair made in a private `gnome-remote-desktop-XXXXXX` temporary directory and imported (`ImportCertificate((sh)(sh))` with both files as descriptors, as g-c-c 50; the directory removed again), then `Enable()`; off = `Disable()`; all interactive (polkit `configure-system-daemon`). Answers `{on, imported}`. |
| `sharing.rdp.system.credentials.set! 🔑 {username, password}` | SetCredentials(a{sv}), interactive. |
| `sharing.ssh.get {}` | installed (`/usr/sbin/sshd` or its units), on (ssh.socket or ssh.service running), enabled, socket and service (exists, fileState, active), installing, command (`ssh <user>@<host>`, GNOME's SSH Login Command). |
| `sharing.ssh.set! {on}` | As GNOME Settings 50 (review fix 1): first polkit's CheckAuthorization of GNOME's own `org.gnome.controlcenter.remote-login-helper` for this process (subject `system-bus-name` = the host's unique name, AllowUserInteraction; auth_admin_keep, its policy implies `systemd1.manage-units` and `manage-unit-files`), so one password dialog covers the systemd calls; dismissed = `core.authCancelled`, refused = `core.notAuthorized`, nothing else called; without that action (no gnome-control-center, no polkit) each systemd call asks itself. Amendment 35: on = StartUnit + EnableUnitFiles of `ssh.socket` when systemd has it, else `ssh.service`; off = DisableUnitFiles of both unit files first, then StopUnit of each (a refusal never leaves a unit enabled for the next boot); systemd's system manager through `call_interactive`. |
| `sharing.ssh.install! {}` | Windows' "Een optionele functie toevoegen": PackageKit CreateTransaction, SetHints (`interactive=true`, `locale=nl_NL.UTF-8`/`en_GB.UTF-8`), Resolve(not installed + this architecture = 262152, `openssh-server`), then InstallPackages(only trusted = 2, the package id), interactive (polkit package-install); its Package/ErrorCode/Finished signals and Percentage changes on a MainContext of its own. Ubuntu's postinst enables and starts ssh.socket; Windows' optional feature arrives stopped, so the install then turns off what came up running or enabled (the switch's authorization and order, review fix 3) and answers `{installed, package, on}` (`on: true` when that was refused). Waits until it finishes (at most 30 minutes; the kit's call waits as long as the host answers), with `sharing.changed {what: "sshInstall", percentage}` meanwhile; one install at a time (`busy sharing.installRunning`). |
| `sharing.media.get {}` | Mediastreaming: available (rygel installed), running (gsd Sharing answers), status (SharingStatus: available, offline, disabled-mobile-broadband, disabled-low-security), currentNetwork, currentNetworkName, carrierType, enabled (rygel shared on the current network), networks (ListNetworks rygel: uuid, name, carrier), folders (rygel.conf MediaExport uris, @MUSIC@/@VIDEOS@/@PICTURES@ expanded). |
| `sharing.media.set! {on}` | gsd Sharing EnableService('rygel') / DisableService('rygel', the current network). |
| `sharing.media.folders.set! {folders}` | `~/.config/rygel.conf` as cc-media-sharing.c writes it: the user's file, else `/etc/rygel.conf`, comments kept; `general` upnp-enabled true, `Tracker` and `Tracker3` enabled false, `MediaExport` enabled true and uris (GKeyFile's list escaping, special folders back to their names). |
| `sharing.media.networks.remove! {uuid}` | DisableService('rygel', uuid). |

Amendment 34: `libgnutls.so.30` is opened with `crate::dlib` (`sharing/tls.rs`): the 14 symbols g-c-c 50 imports for its certificate (privkey init/generate/export2, crt init/set_version/set_serial/set_dn/set_key/activation/expiration/sign/export2, deinit); an RSA 4096 key and a self-signed X.509 v3 certificate `C=US,CN=GNOME`, five years, PEM; the exported data freed through GnuTLS's own `gnutls_free` variable. The key is written 0600 (GNOME's `g_file_set_contents` leaves it at the umask's mode; tighter here, same files).

Events: `sharing.changed {what}` (rdp, system, ssh, sshInstall, media) after every write, and a watcher thread (own MainContext) that turns gsd Sharing's, the RDP daemon's and (review fix 10, on the system bus) the Remote Login configuration daemon's signals into `{what: media}` / `{what: rdp}` / `{what: system}` while a page listens. Errors: stem `sharing` (`host/i18n/sharing.tsv`, records in `tests/i18n/BASELINE-CHANGES.rust-settings-c`): unavailable, badArgs, badCredentials, badFolder, noRemoteDesktop, noRemoteLogin, noTls, certificateFailed, credentialsFailed (params.error: grdctl's own last line, FreeRDP's log lines left out, never a password), noSsh, noPackage, installFailed, installRunning, noSharing, saveFailed; daemon refusals are call_interactive's `core.*`. No new allow row: B0's `allow/network.rs` rows fit (`enable`, `view-only` Write, `screen-share-mode` Read since the review; `tls-cert`, `tls-key` `Service("sharing.rdp.set")`, which the switch writes; gsd's `enabled-connections` stays `Service("sharing.media.set")`, but gsd writes it itself on EnableService/DisableService, as GNOME relies on).

### Native

* `pages/network_advanced.rs` **network-advanced**: "Netwerkadapters", one expander per adapter Windows would list (Ethernet, Wi-Fi, modem, Bluetooth network, WireGuard, and managed virtual devices; never Wi-Fi Direct, veth or unmanaged ones): the kind (with the interface when there are several), vendor and model with GNOME's status, Windows' Uitschakelen / Inschakelen (`net.device.setEnabled`: Device.Disconnect, which blocks autoconnect, is "uitgeschakeld"; Inschakelen insensitive while the cable is out, as GNOME's switch), and "Aanvullende eigenschappen weergeven" (the adapter's own page: Ethernet, the Wi-Fi network's properties or Wi-Fi, Mobiel netwerk, VPN). "Meer instellingen": "Hardware- en verbindingseigenschappen" (every adapter's name, status, connection, MTU and slice 1's Windows property list, with Kopiëren of all), GNOME's "Controle op internetverbinding" (shown only when NetworkManager can check, as GNOME), the link to Geavanceerde instellingen voor delen. Rebuilt when an adapter's state, autoconnect or the connectivity switches change (`net.changed`, compared by digest so a new bit rate does not rebuild it).
* `pages/network_advanced.rs` **sharing-advanced**: Netwerkdetectie (Avahi, information "Aan"), "Mediastreaming" (GNOME's Media Sharing text as the group description; the switch for the current network, its name in the description, insensitive with gsd's status as a notice when sharing is not possible; "Mappen" with Map toevoegen (a folder dialog) and Verwijderen per folder, the special folders with their glyphs; "Netwerken" with Huidig netwerk and Verwijderen), without rygel a card that says so. Follows `sharing.changed media`. Related: Extern bureaublad, Geavanceerd.
* `pages/remotedesktop.rs` **remotedesktop**: the "Extern bureaublad" expander with the switch (`st-remotedesktop-enable`; Windows' "Extern bureaublad inschakelen?" with Bevestigen, then `sharing.rdp.set`; locked `enable` makes it insensitive with the lockdown tooltip; follows `enable` through `crate::gs`), Windows' NLA box (checked, insensitive: grd always asks for credentials), "Poort van Extern bureaublad" with Kopiëren, and GNOME's "Externe besturing" (`st-remotedesktop-view-only`, `gs.toggle(view-only, invert)`, sensitive only while Desktop Sharing is on, as GNOME). "Aanmeldingsgegevens" (GNOME's Login Details for Windows' "Gebruikers van Extern bureaublad"): Gebruikersnaam with Kopiëren, Wachtwoord hidden with Weergeven/Verbergen and Kopiëren, Bewerken (dialog with Tekens weergeven and GNOME's generate button), Nieuw wachtwoord genereren, Versleuteling controleren (GNOME's fingerprint dialog: four lines of eight pairs). "Verbinding maken" (GNOME's How to Connect): Pc-naam and IP-adres (IPv4 first, then global IPv6, from `net.devices`) with Kopiëren. "Extern aanmelden" (GNOME's Remote Login, only when the configuration daemon is there): the switch asks GetCredentials, then a dialog with the stored or GNOME's default details, Bevestigen = SetCredentials, then `sharing.rdp.system.set`; closing the dialog puts the switch back; Poort, Aanmeldingsgegevens (Weergeven and Bewerken with Windows' shield), Versleuteling controleren. Notices: grd not installed, on but not running, GnuTLS missing. Related: Energie en batterij, Geavanceerde instellingen voor delen, Info.
* `pages/system_advanced.rs` **system-advanced**: "Secure Shell": the "OpenSSH-server" expander with its switch (`sharing.ssh.set`, the shield tooltip), Status (socket activation: "start wanneer er een verbinding binnenkomt"), "SSH-aanmeldingsopdracht" with Kopiëren; without openssh-server Windows' "Een optionele functie toevoegen" with Installeren (shield button), a progress ring and "Wordt geïnstalleerd… n%" from the `sshInstall` events, then the page again. Related: Extern bureaublad, Geavanceerde instellingen voor delen.
* `pages/network.rs`: `device_status` and `nm_missing` are `pub(super)` for network_advanced.rs.

Control names: `network-advanced-adapter-<n>`, `-toggle-<n>`, `-properties-<n>`, `-hardware`, `-copy`, `-connectivity`; `sharing-advanced-discovery`, `-media`, `-status`, `-no-rygel`, `-folder-add`, `-folder-remove-<n>`, `-networks`, `-network-remove-<n>`; `st-remotedesktop-enable`, `st-remotedesktop-view-only`, `remotedesktop-nla`, `-copy-port`, `-credentials`, `-credentials-edit`, `-show`, `-copy-user`, `-copy-password`, `-password`, `-generate`, `-verify`, `-copy-name`, `-copy-ip`, `-login`, `-login-switch`, `-login-show`, `-login-edit`, `-login-verify`, `-fingerprint`, `-edit-*`; `system-advanced-ssh`, `-ssh-switch`, `-ssh-status`, `-ssh-copy`, `-ssh-install`, `-ssh-progress`, `-ssh-install-card`.

### plan.json controls dropped or changed (slice 3)

* Remote Login's enable: the plan says "pkexec /usr/libexec/gnome-remote-desktop-enable-service". Its usage is `<pid> <user|headless|system> <true|false>`, it checks the *caller's* pid for `configure-system-daemon` and says "Enabling system service is not meant to be done directly by users"; gnome-remote-desktop's configuration daemon runs it itself (its polkit rule gives the gnome-remote-desktop user YES) when `Enable()` is called, which is what g-c-c 50 calls. So no pkexec runs from WinPoint: Enable/Disable/SetCredentials/GetCredentials/ImportCertificate on the daemon, interactive. The harness and the fakes check that.
* Windows' NLA checkbox is information (always on); Windows' "Gebruikers van Extern bureaublad" (a user list) is GNOME's one set of sign-in details; GNOME's keyring banner "Some settings are locked · Unlock" is not shown: grdctl's lookup asks the keyring to unlock itself, and a failure is the credentials card's description.
* `screen-share-mode` has no control: GNOME has none either (gnome-a.json: "has no UI"); since the review it is a Read row (contract §6.1).
* Windows' Advanced network settings: Netwerk opnieuw instellen (would delete every profile) and Gegevensgebruik (no GNOME counterpart, only NetworkManager's session counters) left out, as windows.json advises; the adapter's "Naam wijzigen" too (NetworkManager has no adapter rename).
* Advanced sharing settings: Windows' per-profile expanders (Privé / Openbaar) do not exist in GNOME (sharing is per network: the Netwerken list); Bestands- en printerdeling and Openbare mappen delen need gnome-user-share (not installed, GNOME hides it too; plan's gnomeFunctionsLeftOut); Netwerkdetectie is information, not a switch (Avahi is a system service with no GNOME control).
* System › Advanced: only GNOME's Secure Shell is there; Windows' Ontwikkelaarsmodus, Taak beëindigen, Sudo, Terminal and the Verkenner options have no GNOME Settings function (windows.json: omit), so they are not offered.
* The SSH install waits for PackageKit in the call (with progress events) instead of a background job; reopening the page during an install shows "Wordt geïnstalleerd…" (`installing`) without a percentage until the next event.

### Notes for I (slice 3)

* No frozen file needed; no new allow row, settings key, registry entry or STEMS change. `common::placeholder` is no longer used by any batch C page (network-advanced, sharing-advanced, remotedesktop, system-advanced were the last four).
* `runtime::is_secret` (amendment 53): `sharing.rdp.credentials.get` (also with `system`), `sharing.rdp.credentials.set` and `sharing.rdp.system.credentials.set` are `"secret": true`. The sharing fake logs SetCredentials' arguments as "(redacted)".
* Kit (not batch C's, a note for A or I): a switch placed on a `SettingsExpander` header does not take a real pointer click; the header's gtk::Button takes the press (the harness's pointer click on the Remote Desktop and OpenSSH Server switches opened the expander and left the switch as it was; the dev channel's click works). Batch C's pages work around it with `remotedesktop::header_switch` (a capture-phase gesture on the header that claims a press on the switch and toggles it). `pages/display.rs:630` and `pages/notifications.rs:51` put a switch on an expander header too and likely have the same problem; the proper fix belongs in `native/kit/src/controls/cards.rs` (`Expander::build`), after which `header_switch` can become `exp.header().set_control(sw.widget())` again.
* Harness (B0's `tests/gnome/harness.py`, seen here): `run/stubs` is made with the caller's umask; with Ubuntu's 002 it is group-writable, so `tools::system_tool` refuses every tool in the harness (the Remote Desktop page shows "gnome-remote-desktop is niet geïnstalleerd" under Aanmeldingsgegevens there, and no grdctl stub call is logged). A `chmod 0700` of `RUN / 'stubs'` in `prepare()` would let scenarios see the stubs' calls.
* Harness: WinPoint's launcher (`host/src/process_scope.rs`) starts apps through systemd's user manager whenever `org.freedesktop.systemd1` is on the session bus; a fake user manager therefore blocks the page relay (`settings_page`). settings-sharing.py starts the user-manager fake only around the Remote Desktop switch (`fakes/c/sharing.py --user-systemd-only`).
* `tests/dbus-fakes/fakes/c/sharing.py` is new and starts in every `run.sh --batch c` run (it owns systemd1 on both private buses, gsd Sharing, grd's two names and PackageKit); slice 1 and 2's eight bus tests still pass with it.
* Not batch C's, as in slices 1 and 2: `services::update::elf::tests::our_binaries_equal_readelf` fails with fresh builds (the libm.so.6 recording).

### Tests (slice 3)

* Host unit tests (`wt/host-tests.sh c --lib`, full run): 639 run, 607 passed, 1 failed (`services::update::elf::tests::our_binaries_equal_readelf`, the libm.so.6 recording of slice 1's note, not batch C's), 31 ignored (27 + the 4 new bus tests); after the last fix of rdp.rs (the descriptor call's reply, below) the filtered run (`-- sharing`) again gave 10 passed, 4 ignored. `services::sharing::tests`: 10 passed, 4 ignored: grdctl's real status output (probed in the dbus-fakes layer with a private gnome-keyring) parsed, VNC's section ignored; a grdctl stand-in shows the password only on standard input and never in argv or an error; credentials and generated passwords; GnuTLS: the SHA-256 vector of "abc", PEM to DER, a generated pair that GIO loads, the key 0600, `C=US`/`CN=GNOME` in the DER, a loading pair kept and a broken one replaced; rygel.conf with comments kept, GNOME's four booleans, the escaped list GKeyFile reads back, special folders both ways; the SSH switch from both units, the package id, the PackageKit filter value; the pages' writes through the allow rows (`tls-cert` refused for `gsettings.set` with `useMethod`, gsd's list only for `sharing.media.set` and only at a valid path); every dangerous method's dry run touches no tool, file or bus, and the refusals (badArgs, badCredentials, badFolder); the reads without the daemons; the sharing texts in both languages.
* D-Bus fakes (`tests/dbus-fakes/run.sh --batch c --strace -- <lib test binary> --ignored --exact …` inside tools/heavy.sh, a fresh set of fakes per test): 12 of 12 passed, strace OK each time: slice 1 and 2's eight with the new fake present, and four new ones against `fakes/c/sharing.py`. Desktop Sharing: on = StartUnit and EnableUnitFiles of the user unit (no password dialog), `enable` true, `tls-cert`/`tls-key` naming the new pair, grdctl asked `status --show-credentials` then `rdp set-credentials <login name>` with a 16-character password on standard input only; get shows the daemon's negotiated port 3390 and the certificate's fingerprint; on again keeps both; off = StopUnit, DisableUnitFiles, `enable` false. Remote Login: GetCredentials, SetCredentials, ImportCertificate (two descriptors; the fake read a PEM certificate and a private key) and Enable, each with ALLOW_INTERACTIVE_AUTHORIZATION (the fake refuses a system write without it); the daemon's fingerprint in the page's form; on again imports nothing; Disable. The first run caught two gio-rs 0.18 bugs on the descriptor call (call_with_unix_fd_list_sync panics on a reply without descriptors; DBusMessage::to_gerror asserts on a method return in debug builds): the call is now a DBusMessage of its own whose reply type is checked here. Secure Shell: not installed, set refused (`sharing.noSsh`), install = SetHints, Resolve(262152, openssh-server) interactive, SetHints, InstallPackages(2, the id) interactive; then StartUnit + EnableUnitFiles of ssh.socket only, interactive; off stops and disables ssh.socket and ssh.service. Media: gsd's status and networks, EnableService('rygel'), DisableService of Kantoor and of the current network, rygel.conf written in the private config directory.
* Native (`wt/native-tests.sh c`): every suite green, 87 passed in total (1 ignored in the kit); win11-gtk-settings 52 passed, 4 new (network_advanced 2, remotedesktop 1, system_advanced 1). Builds (`wt/build.sh c`): OK, no new warnings (host lib 28, win11-launch 1, win11-shell 19, win11-gtk-settings 0, as the base).
* `wt/checks.sh c`: as the base: rustfmt 0 failing (195 files, 165 clean; the 6 new host files clean), check_rust 0 errors / 57 warnings, drift 0 errors / 15 warnings, check_native 0 errors / 48 uncovered, test_native OK (the windows-glossary files well formed), settings-pages OK, release OK; the installer check refuses to run here (XDG_RUNTIME_DIR not 0700), before and after.
* Harness (`tests/gnome/scenarios/settings-sharing.py`, the NetworkManager fake `--system-only` and the sharing fake on the private buses): nl 41 checks, en 45 checks, 0 failed. Each page with its title, no placeholder, no Dutch word in en. Network-advanced: Uitschakelen of wlan0 sent Device.Disconnect with the interactive flag. Sharing-advanced: a real pointer click on Mediastreaming sent EnableService('rygel'); Verwijderen of Kantoor sent DisableService('rygel', Kantoor); Verwijderen of the first folder wrote `uris=@VIDEOS@;@PICTURES@;` and `upnp-enabled=true` to the private rygel.conf. Remotedesktop: a real pointer click on the switch showed "Extern bureaublad inschakelen?", Bevestigen started and enabled the user unit (no password dialog), `enable` true and `tls-cert` the new certificate in the private data directory; a real pointer click on Externe besturing wrote `view-only` false; Extern aanmelden sent GetCredentials, SetCredentials, ImportCertificate and Enable, each interactive. System-advanced: Installeren ran Resolve and InstallPackages (interactive); then a real pointer click on the OpenSSH-server switch started and enabled ssh.socket with the interactive flag. Dialog and plain buttons were pressed through the dev channel (slice 2's note); the switches took real pointer clicks once `header_switch` was in place (the first runs found the expander header taking them). Screenshots `<page>.<lang>.png`, `remotedesktop-on`, `remotedesktop-question`, `remotedesktop-login-question`, `sharing-advanced-on`, `system-advanced-installed` (PNGs not committed).

### Open (slice 3)

* Not run against the real daemons: gnome-remote-desktop's user and system daemons, gsd Sharing with rygel, systemd's ssh units and PackageKit were fakes here, and grdctl was a stand-in (and refused by the harness's tool seam). The certificate GnuTLS makes was checked with GIO, not with a real RDP client; the key size (4096) and the five years are libbonsai's as far as could be seen from g-c-c 50's binary, not its source.
* The keyring prompt: grdctl's credentials lookup may ask gnome-keyring to unlock (a GNOME Shell prompt) and waits up to 5 minutes; GNOME shows a "Some settings are locked · Unlock" banner instead.
* Remote Login's switch reads the stored details with GetCredentials, so turning it on asks for the administrator password before the dialog shows (auth_admin_keep then covers SetCredentials, ImportCertificate and Enable).
* File Sharing (gnome-user-share, not installed) and Windows' per-profile sharing switches have no page controls; Windows' Developer Mode, End task, Sudo and Terminal rows are not offered (no GNOME Settings function).

## Harness step: settings-network.py, settings-sharing.py, tests/gnome/lib/standins/c.py

plan.json "testing" (GNOME HARNESS) for batch C's fourteen pages, in the private GNOME (`tests/gnome/harness.py`,
through `wt/harness.sh c`, so from a tmpfs copy inside the heavy lock), each scenario with `--lang nl` and `--lang en`
and with `--strace` (the pages reach systemd, PackageKit, grdctl and, in their fallbacks, nmcli and rfkill).

### The stand-ins (`tests/gnome/lib/standins/c.py`)

* `GSD_RFKILL`, `GSD_RFKILL_HARDWARE`: gsd's `org.gnome.SettingsDaemon.Rfkill` on the private session bus as `Spec`s for
  B0's builder (`lib.standins.start`): flight mode off with AirplaneMode, BluetoothAirplaneMode and WwanAirplaneMode
  writable; the second with HardwareAirplaneMode on.
* `start_fake(h, stem, *options, tag=None)`: batch C's stateful daemons from `tests/dbus-fakes/fakes/c/` (NetworkManager,
  ModemManager, the sharing daemons) as harness stand-ins (a child of the harness on its private buses, "ready" when
  it owns its names), returned as B0's `StandIn` (`calls()`, `stop()`). B0's builder answers fixed values only; these
  fakes change their state with the writes, refuse NetworkManager's and the daemons' wrong signatures, and refuse a
  system write without the password-dialog flag. `calls_after(standin, before, method, n, timeout)` waits for calls.

### What the scenarios check

Per page: opened with `h.open('settings:<id>')` (Wi-Fi-eigenschappen through `h.settings_data_page('wifi',
'wifi-active-properties', title='Lab-5G')`), the header against registry.rs (title, or the crumb-<id> chain plus the
current crumb, and that the current crumb is not cut to "…"), Settings in the run's language, no `st-placeholder`, no
Dutch word in en, a screenshot `<id>.<lang>` (more: `<id>-<what>.<lang>`). Controls get real pointer clicks; a write
must reach its daemon with GNOME's value and, where the daemon checks polkit, ALLOW_INTERACTIVE_AUTHORIZATION.

settings-network.py:
1. No daemon: Netwerk en internet shows GNOME's "Netwerk niet beschikbaar"; `caps.get` reports no modem, so "Mobiel
   netwerk" is not offered, and its deep link shows GNOME's "no mobile broadband adapter" page; the flight mode switch
   without gsd's Rfkill runs the run/stubs `nmcli radio all off` (nothing real) and goes back after the refusal.
2. NetworkManager + gsd Rfkill: the overview's rows (Wi-Fi, Ethernet, VPN, Mobiele hotspot with its switch,
   Vliegtuigmodus, Proxy, Geavanceerde netwerkinstellingen; no Mobiel netwerk); Wi-Fi: NetworkManager's access point
   listed, the switch sets `WirelessEnabled` false then true (interactive); Wi-Fi-eigenschappen: autoconnect false,
   permissions `['user:<login>:']`, metered 1, assigned-mac-address `random`, each one Update2 that keeps zone,
   x-vendor and seen-bssids (interactive); Bekende netwerken beheren; Ethernet: eth0's switch insensitive without a
   cable (GNOME's), metered 1 on the wired profile; VPN: Verbinding maken = ActivateConnection of Kantoor
   (interactive), its "Beschikbaar maken voor andere gebruikers" = Update2 permissions; Vliegtuigmodus: gsd's
   AirplaneMode true then false, and with the hardware switch GNOME's notice and an on, insensitive switch; Proxy:
   automatic detection = mode auto with an empty autoconfig-url and the WPAD warning, mode none again, outside writes
   of mode move the switch both ways, the script dialog writes the URL then mode auto, the server dialog writes HTTP,
   HTTPS and FTP `proxy.example.org:8080`, ignore-hosts `['localhost', '127.0.0.1', '*.local']` then mode manual, no
   authentication-password; Mobiele hotspot: GNOME's question, then AddAndActivateConnection2 (Hotspot, mode ap, IPv4
   shared, interactive), the proposed password shown.
3. ModemManager too (Settings closed and opened again, so its backend asks the bus): caps report the modem, Mobiel
   netwerk is offered on the overview and on Vliegtuigmodus; the mobile page shows the locked SIM, Ontgrendelen (a real
   click) and the PIN dialog send SendPin('1234') (interactive), then Mobiele gegevens off = Update2 autoconnect false
   of the SIM's gsm profile and Roamen = Update2 home-only false (both keep apn and sim-id).

settings-sharing.py:
1. No daemon: Geavanceerde netwerkinstellingen shows "Netwerk niet beschikbaar"; Geavanceerde instellingen voor delen
   says gnome-settings-daemon isn't running, its Mediastreaming switch insensitive.
2. Geavanceerde netwerkinstellingen with NetworkManager: the adapters, Hardware- en verbindingseigenschappen, the
   connectivity check (ConnectivityCheckEnabled false then true, interactive), Uitschakelen of wlan0 =
   Device.Disconnect (interactive), now with a real click.
3. Geavanceerde instellingen voor delen with gsd Sharing: Mediastreaming = EnableService('rygel'); Netwerken opened
   with a real click on its header, Verwijderen of Kantoor = DisableService('rygel', Kantoor); Verwijderen of Music
   rewrites rygel.conf (`uris=@VIDEOS@;@PICTURES@;`, `upnp-enabled=true`).
4. Extern bureaublad: off at first; the switch asks "Extern bureaublad inschakelen?", then StartUnit +
   EnableUnitFiles of the user unit (no password dialog: a user unit), `enable` true, `tls-cert` the new certificate,
   its key file 0600; grdctl is the run/stubs one (`status --show-credentials`), never with a password in argv;
   Externe besturing writes view-only false; an outside write of view-only true turns it off, of enable false turns
   the main switch off and Externe besturing insensitive (GNOME's); Extern aanmelden = GetCredentials, SetCredentials,
   ImportCertificate, Enable, each interactive.
5. Systeem › Geavanceerd: "Een optionele functie toevoegen" without openssh-server; Installeren = Resolve and
   InstallPackages of openssh-server (interactive); then the OpenSSH-server switch = StartUnit + EnableUnitFiles of
   ssh.socket (interactive).

Both: `allow/network.rs` has no Owned key (no control here points to a WinPoint page instead), and the sandbox has no
`/etc/glib-2.0` locks (README; lockdown stays a host unit test).

### Page fixes the harness found (batch C files)

* **Wi-Fi switch** (`host/src/services/system/network.rs`, amendment 57's file): `system.network.setWifi` ran
  `nmcli radio wifi`; it is now GNOME's Properties.Set of NetworkManager's `WirelessEnabled` through `call_interactive`
  (polkit's enable-disable-wifi), nmcli only while NetworkManager is not on the bus. Bus test
  `services::system::network::tests::bus_wifi_switch_sets_wireless_enabled` (fakes/c/networkmanager.py).
* **Wi-Fi list** (`pages/network.rs` `WifiPage::load`): with one adapter the page listed nmcli's networks
  (`system.network.wifiList`), so in the harness it said "Geen netwerken gevonden" while NetworkManager listed Lab-5G.
  It now always takes NetworkManager's access points over D-Bus (`net.devices {accessPoints, scan}`, as GNOME's Wi-Fi
  panel), the first adapter's when there is one adapter; nmcli's list only when net.devices has no Wi-Fi adapter.
* **Wi-Fi-eigenschappen rebuilt itself after its own writes** (`pages/wifi_network.rs`, `net_editor.rs`): its
  net.changed follower saw the profile differ and rebuilt the page about a second after each switch, which swallowed
  the next click (permissions and the MAC drop-down were lost, metered flipped twice) and scrolled the page to the top.
  `ed::save_keys` now notes the profile it wrote; within 3 s the follower takes the profile it reads back as the new
  state instead of rebuilding (a change by another program still rebuilds). Native test
  `net_editor::tests::a_pages_own_write_is_recognised_for_its_profile_only`.
* **Buttons on expander headers** (`pages/remotedesktop.rs` `header_button`, used by vpn.rs, network.rs,
  wifi_network.rs, mobile.rs, network_advanced.rs, remotedesktop.rs): the kit's expander header is a gtk::Button whose
  click gesture runs in the capture phase, so a real click on a button placed on the header (Verbinding maken,
  Uitschakelen, Kopiëren, Map toevoegen, Profiel toevoegen, APN toevoegen, Bewerken, Vernieuwen) opened the expander
  and never clicked the button (the harness's click on Kantoor's "Verbinding maken" did that). `header_button`, like
  slice 3's `header_switch`, puts a capture-phase gesture on the header that claims a press on the button and clicks it
  on the release.
* **Proxy server dialog** (`pages/proxy.rs`): the exceptions text was kit::field's one-line header, cut to "…die
  beginnen met de volge…"; it is now a wrapping header, in full as Windows shows it.

### Known failures (not batch C's files)

* `B-stubs-umask` (batch B's note 10): run/stubs is 0775 under Ubuntu's umask, so `tools::system_tool` refuses it; the
  scenarios record it and chmod the directory themselves.
* `C-dialog-click` (batch B's note 9, the same cause): a real click never reaches a widget inside a
  `kit::controls::Dialog` (its modal layer's GestureClick claims every press); the switches, check box and buttons in
  the proxy, hotspot, PIN and Remote Desktop dialogs get a real click first and then the dev channel's click, and every
  dialog action was still checked. Real clicks on page widgets (also the info bar's Ontgrendelen) work.
* `B-crumb-ellipsis` (batch B's note 11) on Geavanceerde instellingen voor delen: "Netwerk en internet ›
  Geavanceerde netwerkinstellingen › …", the current crumb 28 px wide. Windows keeps the current page and collapses
  the leading items.

### Results

Final runs on this branch's last build (16-11, `--source worktree`, `--strace`; every failure a known one):

| run | checks | passed | failed | strace |
|-----|--------|--------|--------|--------|
| settings-network.py nl (23:39) | 183 | 181 | 2: B-stubs-umask, C-dialog-click (0 of 7 real) | connect OK (287), exec OK (448) |
| settings-network.py en (22:52) | 194 | 192 | 2: B-stubs-umask, C-dialog-click (0 of 7 real) | connect OK (287), exec OK (448) |
| settings-sharing.py nl (21:59) | 86 | 82 | 4: B-stubs-umask, B-crumb-ellipsis (twice), C-dialog-click (0 of 2 real) | connect OK (198), exec OK (276) |
| settings-sharing.py en (22:18) | 92 | 88 | 4: the same | connect OK (198), exec OK (278) |
| native-shell.py nl (regression) | 21 | 21 | 0 | |
| lock-unlock.py nl (regression) | 5 | 5 | 0 | |

The first network run of this step (20:54, nl, before the fixes above) had 173 checks and 9 failures, 7 of them not
known: the lost clicks on Wi-Fi-eigenschappen (permissions, the MAC drop-down), Verbinding maken of Kantoor, and a
wrong expectation of the scenario (a new hotspot shows its proposed password, so no "Wachtwoord weergeven"). In en no
page has a Dutch word (11 page checks in the network run, 6 in the sharing run). The regression runs wrote to untracked
directories (`tests/gnome/scenarios/c-regress-*.out`, not batch C's files), removed afterwards; the strace logs stay
local (each `results.json` has its `straceConnect` and `straceExec` verdicts).

Unit and bus tests after the fixes: host `wt/host-tests.sh c --lib` 640 run, 607 passed, 32 ignored, 1 failed (the
libm.so.6 readelf recording of slice 1's note, not batch C's); bus tests (`tests/dbus-fakes/run.sh --batch c --strace`
inside tools/heavy.sh) `bus_wifi_switch_sets_wireless_enabled` and `bus_flight_mode_goes_through_gsd_rfkill` passed,
strace OK; native `wt/native-tests.sh c` 88 passed, 1 ignored (Settings 53, one new); builds without new warnings;
`wt/checks.sh c` as the base (rustfmt 0 failing, check_rust 0 errors / 57 warnings, drift 0 / 15, check_native 0 / 48
uncovered, test_native, settings-pages and release OK; the installer check refuses to run here).

### Notes for I (harness step)

* Kit, `native/kit/src/controls/cards.rs` (`Expander::build`): a control on an expander header never gets a real click
  (the header gtk::Button's capture-phase click gesture takes it). Batch C works around it per control
  (`remotedesktop::header_switch`, `remotedesktop::header_button`); the proper fix is in the kit: a capture-phase
  GestureClick on the header that claims a press inside `card.control_box()`'s bounds (and lets the control handle
  it), after which both helpers become `exp.header().set_control(...)` again. Outside batch C the same pattern is in
  about.rs, background.rs, colors.rs, display.rs, notifications.rs and update.rs (`header().set_control`).
* B's notes 9 (dialog layer), 10 (run/stubs mode) and 11 (breadcrumb) also cover batch C's known failures; with them
  applied, `C-dialog-click`, `B-stubs-umask` and `B-crumb-ellipsis` in these scenarios should pass.
* No frozen file changed; no new UI string (the glossary is unchanged).

### Open (harness step)

* The harness does not click through the IP, DNS, security, 802.1X, new-network, VPN add/import, hotspot edit, APN and
  PIN-change dialogs (their pure helpers and the host merge tests cover them), nor a scan of mobile networks.
* Nothing ran against the real daemons; the stand-ins are the fakes of slices 1 to 3 and gsd Rfkill from B0's builder.


## Codex continuation, 17-11-2026: static review only

Starting point: WIP 737ca765 and helpers/c-review-findings.json. No build, cargo test, D-Bus fake run or GNOME
harness was started: Robin's session must remain untouched and the memory check found only 3.5 GiB available RAM
with 6 GiB swap already occupied. The historical results above do not validate the WIP or this continuation.

The 13 requested repairs are present in the WIP at source level:

* Host/safety 1: ssh::authorize checks GNOME's polkit action; off disables the unit files before stopping units.
* Host/safety 2: the five no-control proxy/RDP keys are Read, not Write. The frozen NO_CONTROL audit additions
  remain an integration task, as previously noted.
* Host/safety 3: the PackageKit fake now starts/enables ssh.socket; after_install turns it off, or the install
  answer reports on=true when cleanup failed. The updated bus test has not been run.
* Page parity 1: new enterprise/WEP networks use the security form; system.network.connect now searches stored
  profiles before refusing unsaved enterprise/WEP. The Wi-Fi row still uses this host path, rather than direct
  net.connection.activate by UUID; duplicates with the same SSID and missing-secret agents still need checking.
* Page parity 2: overview and hotspot page share the confirmation flow, and overview follows hotspot events.
* Page parity 3: Wi-Fi follows active/devices/accessPoints/connections, with a delayed reload and page refresh when
  the active profile UUID changes.
* Page parity 4: the host caches the proposed key; the page hides the QR unless the hotspot is active.
* Page parity 5: flight mode follows state/modem/radios and system.network.changed.
* Page parity 6: mobile registers its follower before status-page returns.
* Page parity 7: sharing watches the system configuration bus; Remote Login compares its own shape.
* Page parity 8: Ethernet rereads displayed profiles on connections events, ignoring timestamp-only changes and
  briefly suppressing the page's own writes.
* Page parity 9: the connected-properties card distinguishes open/OWE from secured profiles.
* Page parity 10: adapter selection updates the hardware slot and the rows used by its copy button.

Two follow-up source corrections:

* new_network_with preserves leading/trailing spaces in an SSID instead of silently connecting to a different
  name. This is especially relevant to the new fixed-SSID enterprise/WEP dialog.
* hotspot::toggle forwards the interface returned by net.hotspot.get into net.hotspot.start. The selected adapter
  is kept across the confirmation instead of being selected again implicitly by the host.

Light checks completed:

* tools/rustfmt-check.sh: 195 host files, 0 failing (30 pre-existing formatting exceptions).
* tools/i18n/check_pairs.py: 0 errors, 0 warnings.
* tools/settings-pages.py: exit 0.
* tools/i18n/check_native.py before and after the follow-up: 0 errors, the same 48 native-dutch-only warnings.
* tools/check-no-webkit.py: OK, 36,728 files, 19 existing binaries checked. These binaries predate this continuation;
  this does not validate linking or runtime behaviour of the changed source.
* Both edited Rust files parse with rustfmt --emit stdout --config skip_children=true; git diff --check passes.

Still required before C can be called complete or merged: release builds through tools/heavy.sh, host/native/bus
regression tests, settings-network.py and settings-sharing.py in nl/en, and the post-fix review. Include SSIDs
with surrounding spaces and a hotspot confirmation with two adapters. Do not install into or operate Robin's
live session. Nothing has been pushed.


## Integrated follow-up, 17 November 2026 (supersedes the two import/APN limitations above)

VPN import now parses offline through libnm and the installed plugin's importer, or NetworkManager's native
WireGuard importer. It creates no NMClient and starts no connection. The host saves the resulting typed settings
with its existing interactive AddConnection2 path; the API manifest marks this method secret. The plugin description
comes only from the installed system inventory and libnm retains its module ownership validation. No editor factory
is called, no crate was added and the tested OpenVPN import module has no GTK dependency.

The APN form now also supplies the initial EPS bearer's user name, newly entered password/flags and authentication
refusal flags. NOAUTH retains NetworkManager's normal remote-peer behaviour; it does not select the client's PAP/CHAP
method. An empty password edit preserves the existing internet and attach passwords independently. The form represents
one set of newly entered credentials for both bearers; it does not expose separate internet/attach credential editors.
An existing internet-only password must be entered again to copy it to a newly enabled attach bearer.

Evidence: integrated host/native release builds passed; 824 host tests and 250 native tests passed. All 16 C private-bus
cases passed, including real offline WireGuard and OpenVPN import followed by interactive AddConnection2. The OpenVPN
test copies the read-only system module into its private namespace, where it is owned by the test uid, without weakening
libnm's production ownership checks. Every successful traced connection stayed on the private buses. Real daemon,
polkit prompt and modem/SIM checks still require hardware or a separate authorised live installation.

API references: https://networkmanager.dev/docs/api/latest/settings-gsm.html and
https://networkmanager.dev/docs/libnm/latest/NMVpnPluginInfo.html .
