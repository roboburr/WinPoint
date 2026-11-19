# Settings parity, batch B: Bluetooth, printers & colour (handoff note)

Contract: notes/claude/settings-parity.md (sections 1, 2, 3, 4.3, 5.3, 5.8, 6, 7). Worktree
`/run/media/robin/22TB/win11-gnome-wt/b`, branch `claude/settings-b`, based on af8eebdc. The batch runs in two
slices in this worktree: slice 1 (this section) built bluetooth, bluetooth-devices, autoplay and usb; slice 2 builds
printers, printer and colorprofile (`printers.*`, `devices.color.*`, tests/gnome/lib/fake_ipp.py).

## Slice 1: bluetooth, bluetooth-devices, autoplay, usb

### Host

`host/src/services/devices.rs` (dispatch, the watcher, the dry run) with `devices/{bluetooth,batteries,thunderbolt}.rs`,
`devices/tests.rs` and the test-only `devices/fake_bus.rs`. Methods (all in `METHODS` and
`host/src/api-methods/devices.json`):

| Method | ! | What it does |
|---|---|---|
| `devices.bluetooth.adapter {}` | | the first BlueZ adapter (the lowest path, as `system.bluetooth.get`): `{available, path, address, alias, name, powered, discoverable, discoverableTimeout, pairable}` |
| `devices.bluetooth.setDiscoverable {on}` | ! | GNOME's "Visible as" while the page shows, as a lease: on sets Discoverable (when powered and not discoverable yet), the page renews it every 60 s, the host ends it on `{on:false}` (the page sends it when it goes and, waited for, when its window closes), after 150 s without a renewal while the backend lives on (a one-shot source at the lease's end) and in `shutdown` (which a SIGTERM'd backend reaches only with note 13 for I). It only turns off a Discoverable it turned on. |
| `devices.bluetooth.sendFiles {address, name?}` | | GNOME's Send Files: `bluetooth-sendto --device=<address> --name=<name>` through `tools::system_tool` (no `LC_ALL=C`: it is a window), reaped by a thread; no tool: `unsupported devices.sendFilesMissing` |
| `devices.batteries {}` | | UPower's EnumerateDevices without line power and the computer's own batteries (PowerSupply), so GNOME's Power › Connected Devices: `{available, devices: [{id, path, kind, model, vendor, percentage, level, state, icon, address, nativePath}]}`; `address` is the Bluetooth address from the bluez backend's `/org/bluez/hciN/dev_..`, `hid-..-battery` or the HID serial |
| `devices.thunderbolt.get {}` | | bolt's Manager (AuthMode, Probing, DefaultPolicy), the domains' SecurityLevel and the devices without the hosts: `{uid, name, vendor, type, status, authorized, connected, stored, pending, policy, key, parent, domain, generation, connectTime, authorizeTime, storeTime}` |
| `devices.thunderbolt.setAuthMode {mode}` | ! | Properties.Set AuthMode enabled/disabled, `call_interactive` (bolt.manage) |
| `devices.thunderbolt.enroll {uid, policy?}` | ! | EnrollDevice(uid, policy (default "default"), ""), `call_interactive` (bolt.enroll) |
| `devices.thunderbolt.authorize {uid}` | ! | DeviceByUid, then Device.Authorize(""), `call_interactive` (bolt.authorize) |
| `devices.thunderbolt.forget {uid}` | ! | ForgetDevice(uid), `call_interactive` (bolt.manage) |

Every method checks its arguments first, also in a dry run (`bad_args devices.badArgs` / `devices.badDevice`);
under `ctx.dry_run` a dangerous method answers `{ok, dryRun}` and calls nothing. Without a daemon the reads answer
`{available: false}`, the writes `unavailable` (`devices.noBluetooth`, `devices.thunderboltUnavailable`); an unknown
uid is `not_found devices.thunderboltNotFound`; polkit's answers are `call_interactive`'s (`core.notAuthorized`,
`core.authCancelled`). Events: `devices.changed {what}` with `batteries`, `thunderbolt` or `bluetooth`, from a watcher
thread (own MainContext, asleep in `iteration(true)` while nothing happens) that subscribes to every signal of UPower,
bolt and BlueZ: the first signal of a quiet kind arms a one-shot 250 ms source that emits every dirty kind once; after
every write too. Stems: `host/i18n/devices.tsv` (8 rows, `#@ new`, keys in
`tests/i18n/BASELINE-CHANGES.rust-settings-b`). `allow/devices.rs`: the autoplay page writes exactly its four
media-handling keys (autorun-never and the three autorun lists); review B took `automount-open` out (nothing reads it).

Amendment 57 (`host/src/services/system/bluetooth.rs`): `set_powered` first goes through gsd's Rfkill on the session
bus as GNOME Settings' switch does (`BluetoothAirplaneMode = !on`, cc-bluetooth-panel.c): `gsd_step` decides (no gsd
or no `BluetoothHasAirplaneMode`: BlueZ and rfkill as before; on while `BluetoothHardwareAirplaneMode`:
`blocked system.bt_hard_blocked`; already so: on goes on with BlueZ's Powered, off is done when the adapter is off),
then `wait_power` (which powers an unblocked adapter that BlueZ left off); when gsd refuses the write or the adapter
stays off the old path runs. `system.bluetooth.get` gained `airplane` (gsd AirplaneMode) and `hardwareAirplane`
(BluetoothHardwareAirplaneMode), null without gsd. Every 3b898e6 literal of the file is still there (drift), the
function names are kept (allow-host-r2a.txt names `fn:get`), `set_powered`'s signature is the one network.rs calls.

### Native

* **bluetooth** (extended, bluetooth.rs; home.rs' helpers unchanged, `pages/mod.rs` `home_api` compiles): "Detecteerbaar
  als “{alias}”" with BlueZ's Alias (devices.bluetooth.adapter, hostname as fallback) and the discoverable lease
  (asked when the page opens or Bluetooth is switched on, renewed every minute, given up in cleanup); device tiles
  take UPower's battery where BlueZ has none (`with_battery`, followed through `devices.changed`); GNOME's "Airplane
  Mode is On" as an info bar with "Vliegtuigmodus uitschakelen" (`system.network.setAirplane {enabled:false}`, C's
  method) and "Hardware Airplane Mode is On" (the switch insensitive); Windows' rows under the Bluetooth card from
  the registry (`common::child_ids("bluetooth")`: Apparaten, Printers en scanners, Muis, Touchpad, Pen, Automatisch
  afspelen, USB, caps-filtered), also in the no-Bluetooth state, so the mouse page is reachable on a computer
  without Bluetooth (Robin: "muis settings mist"). Helpers for bt_devices.rs became `pub(super)` (`text_of`,
  `upper_addr`, `dev_name`, `kind_label`, `attempt`, `remove_device`, `open_add_device_then`, `CloseFn`) and new:
  `battery_for`, `with_battery`, `battery_list`.
* **bluetooth-devices** (bt_devices.rs): Windows' Devices page: "Een apparaat toevoegen" (the existing dialog), the
  groups "Muis, toetsenbord en pen", "Audio", "Overige apparaten"; a device is Windows' card with its status,
  battery and "⋯" menu: Verbinding maken / Verbinding verbreken, Eigenschappen and Apparaat verwijderen (Windows'
  question). Eigenschappen is GNOME's device dialog: Gekoppeld, Type, Adres, Batterij, GNOME's settings shortcut
  (Muis- / Toetsenbord- / Geluidsinstellingen) and "Bestanden verzenden" (phones and computers). UPower devices that
  belong to no listed Bluetooth device are cards of their own (battery, charging). Notices: no
  Bluetooth, Bluetooth off (link to Bluetooth en apparaten), no devices. Related: Geluid, Printers en scanners.
* **autoplay** (autoplay.rs): "Automatisch afspelen gebruiken voor alle media en apparaten" (`gs.toggle_card`
  autorun-never inverted); "Standaardinstellingen voor Automatisch afspelen kiezen": Verwisselbaar station (GNOME's
  fixed "Altijd vragen": gnome-shell asks for a drive without a content type, no key changes that), Geheugenkaart
  (x-content/image-dcf), Audio-cd, Dvd-video, Muziekspeler, Software and "Andere
  mediatypen" (GNOME's Other Media Types dialog: Type + Actie, 12 types, names from apps.forType). A media choice
  (Altijd vragen / Geen actie ondernemen / Map openen om bestanden weer te geven / an app of apps.forType) moves the
  type into one of the three lists in one `gsettings.set` (GNOME's autorun_set_preferences), an app also becomes the
  type's default (`apps.setDefaultForType`, as g_app_info_set_as_default_for_type). The rows follow
  `gsettings.changed`, are insensitive while AutoPlay is off (GNOME greys them; Verwisselbaar station too, as
  Windows) and show the lock tooltip when a key is not writable. Related: Standaard-apps.
* **usb** (usb.rs): "Directe toegang" (bolt AuthMode, the admin tooltip) and "USB4-hubs en -apparaten" with GNOME's
  Pending Devices first; each device an expander (open by itself for a new pending device) with "Afhankelijk van"
  (GNOME's Depends on: the unauthorised devices it hangs behind, named before the button that authorises them too),
  "Nog niet geautoriseerd · Autoriseren en verbinden", Status, Fabrikant, UUID and the times; "Autoriseren en verbinden" (shield; the device and its unauthorised parents root first, EnrollDevice for a new one, Authorize for a
  stored one, as cc-bolt-device-dialog.c), insensitive while Directe toegang is off, and "Apparaat vergeten" (shield,
  with a question). Notices: firmware without security (none/dponly/usbonly), bolt not running. Live through
  `devices.changed {what: thunderbolt}`.

Strings: `tests/i18n/windows-glossary/b.tsv` (all `verified=no`; Windows' own where windows.json names one, else
winpoint or rename).

### plan.json controls dropped or changed

* bluetooth "Detecteerbaar als": Discoverable only, not Pairable. GNOME's setup mode also sets Pairable and its
  settings widget registers a pairing agent; WinPoint has an agent only while its own dialog pairs, so an incoming
  pairing request would have nobody to answer it. BlueZ's DiscoverableTimeout is left as it is (changing it is an
  adapter-wide setting GNOME does not write); the host lease is the safety net the plan asked for.
* bluetooth "Vliegtuigmodus uitschakelen": shown only while gsd reports flight mode (the fields are null without gsd,
  so no banner on Hyprland).
* bluetooth-devices: Windows' related "Bestanden verzenden of ontvangen via Bluetooth" is not a page link: receiving
  needs gnome-user-share (not installed) and sending needs a device, so "Bestanden verzenden" is in each phone's and
  computer's dialog (system.bluetooth.get lists no UUIDs, so Object Push support cannot be tested per device).
  Swift Pair, downloads over metered connections and the discovery mode: no GNOME function (windows.json: omit).
  No settings shortcut for pens (GNOME has Mouse, Sound and Keyboard only); the keyboard goes to keyboard-advanced.
* autoplay: "Verwisselbaar station" is Windows' first row; windows.json mapped it to automount-open, but nothing in
  GNOME 50 reads that key (libshell-18's AutorunDispatcher reads autorun-never and the three lists only and asks for
  a mount without an x-content type; no binary under /usr/bin, /usr/lib or /usr/libexec names automount-open,
  Nautilus included) and GNOME Settings 50 has no control for it. Review B: the row shows that behaviour as a fixed
  value ("Altijd vragen", no drop-down), greyed with the rows while AutoPlay is off; automount-open left
  `allow/devices.rs`. I or Robin may still drop the row. Windows' "(Verkenner)" after "Map openen om bestanden weer
  te geven" is left out because GNOME opens the default file manager (b.tsv: rename).
* usb: Windows' "USB-meldingen" and "USB-batterijbesparing" have no GNOME function (omitted, as windows.json says);
  the hosts are not listed (GNOME lists peripherals only).

### Tests

* Host (`tests/rust-sandbox`, `services::devices::tests` 12, `services::system::bluetooth::tests` 2 new): UPower
  items and addresses, bolt device decoding and argument checks, the adapter, send-files arguments and a stub
  bluetooth-sendto that records its argv, and against the in-process D-Bus fake on the private session bus
  (`devices/fake_bus.rs`: objects, properties, a message filter that records every call with its interactive flag):
  bolt reads (no password) and every write with ALLOW_INTERACTIVE_AUTHORIZATION and its exact arguments, polkit's
  refusal, an unknown uid, no bolt; UPower batteries; the discoverable lease (one write, the renewal writes nothing,
  off undoes only its own); the dry run calls no daemon; gsd Rfkill read and the Set of BluetoothAirplaneMode.
  tests/dbus-fakes (the bubblewrap layer) is not used: the in-process fakes run inside the normal host test run.
* Native: bt_devices (groups, settings shortcuts), usb (the authorise chain, firmware security), autoplay (GNOME's
  choice of a type, a choice moving the type into one list).
* Harness: `tests/gnome/scenarios/settings-devices.py` with the stand-ins of `tests/gnome/lib/standins/b.py` (bolt,
  UPower), `--source worktree`, 16-11 11:10 (nl, `settings-devices.out`) and 11:22 (en, `settings-devices.en.out`):
  19 of 19 checks each. Bluetooth en apparaten without BlueZ lists its sub-pages; AutoPlay's switch writes
  autorun-never, an outside write of autorun-x-content-ignore moves the Geheugenkaart row, the rows grey out with
  AutoPlay off; Apparaten shows the stand-in's wireless mouse (55 %) and not the laptop battery; USB shows AuthMode,
  the pending dock's real-clicked "Autoriseren en verbinden" sends `EnrollDevice('<uid>', 'default', '')` with
  interactive authorization, the switch off sends Set AuthMode 'disabled'. The first run (10:40) failed the two
  enrolment checks while the button was on the expander header (note 4 for I); slice 1's last commit moved it.

### Notes for I

1. registry.rs (frozen): Windows' description of Apparaten is "Muis, toetsenbord, pen, audio, beeldschermen en docks,
   overige apparaten" / "Mouse, keyboard, pen, audio, displays and docks, other devices" (windows.json); the registry
   has "Gekoppelde apparaten, verbinden en verwijderen". Proposed patch in `page!("bluetooth-devices", ...)`: the
   Windows pair above. Automatisch afspelen: Windows has "Standaardinstellingen voor verwisselbare stations en
   geheugenkaarten" / "Defaults for removable drives and memory cards" (now "Wat er gebeurt als je een usb-stick of
   cd plaatst").
2. C (network.rs) routes `system.network.setAirplane` through gsd's AirplaneMode (amendment 57); with that, the
   Bluetooth page's "Vliegtuigmodus uitschakelen" is GNOME's "Turn Off Airplane Mode" exactly. `set_airplane`'s
   fallback `bt.set_powered(false)` now goes through gsd first too, which is what GNOME would do.
3. Step I's check for UI strings: every `t(nl, en)` pair of the six page files is in a windows-glossary file
   (`tests/i18n/windows-glossary/b.tsv`, or b0.tsv) or was in native/ at af8eebdc (review B: 17 pairs were missing,
   now 518 of 518; the scan is in the review section below).
4. Kit (A's or I's): a `gtk::Button` put on a `SettingsExpander` header (`header().set_control`) never gets its click:
   the harness's real click on the USB page's "Autoriseren en verbinden" in the header toggled the expander and sent
   nothing (run of 16-11 10:40; GtkButton's click gesture runs in the capture phase, so the outer header button
   claims the press first). Batch B moved its buttons out of headers. Other pages that put a button in an expander
   header and may have the same problem: about.rs (Kopiëren), network.rs (refresh, copy), update.rs; a switch or
   the kit's Dropdown there (display.rs night light, notifications.rs, background.rs, colors.rs) needs a real-click
   check too. A fix in kit/src/controls/cards.rs: make the header a box with its own click gesture (bubble phase)
   instead of a gtk::Button, or stop a control's press from reaching the header.
5. `services::update::elf::tests::our_binaries_equal_readelf` fails on every fresh release build of af8eebdc, not
   because of batch B: B0's `settings_mirror::oklab` (`((c + 0.055) / 1.055).powf(2.4)`) is the only caller of
   `f64::powf` in the host (objdump of the test binary), so `win11-app` now has `NEEDED libm.so.6` (`pow@GLIBC_2.29`;
   `cbrt` comes from compiler_builtins). Binaries without libm predate a rebuild with B0 (the main checkout's
   win11-app is from 02:38, B0's mirror from 07:01); batch E's fresh build has libm too. Fix for I (one of): add `"libm.so.6"`
   before `"libc.so.6"` in the recorded list of win11-app in `host/src/services/update/elf.rs` (libm is part of
   glibc, so nothing new is installed), or give settings_mirror.rs an sRGB curve without a libm call.

### For slice 2 (printers, printer, colorprofile)

* `devices.color.*` goes into `host/src/services/devices.rs` (`METHODS`, `check`, `run`, `Endpoints` gains colord's
  name) with a `devices/color.rs`; `printers.*` into `host/src/services/printers.rs`; both fragments' rows into
  `host/src/api-methods/devices.json` (it holds `devices.` and `printers.`), the texts into `host/i18n/devices.tsv`
  and `printers.tsv` (`#@ new` and a `key:` line in `tests/i18n/BASELINE-CHANGES.rust-settings-b`).
* `devices/fake_bus.rs` serves any interface from XML (properties, methods, recorded calls with the interactive
  flag) on the sandbox's private session bus; `devices/tests.rs` shows its use.
* The watcher already emits `devices.changed`; a colord subscription fits its list.
* The scenario and stand-ins (`tests/gnome/scenarios/settings-devices.py`, `tests/gnome/lib/standins/b.py`) are
  slice 1's; the printer and colour steps append to `run(h)`.
* Keep buttons off expander headers (note 4 for I).

## Slice 2: printers, printer, colorprofile

### Amendment 41: how a driverless (IPP Everywhere) queue is made, verified 16-11

g-c-c 50 has no `everywhere` literal: it lets system-config-printer's Python `GetBestDrivers` pick the driver (the
plan drops that service). The experiment, in a network-less bubblewrap sandbox (`--unshare-net`, `--tmpfs /run`: no
system bus, no real cupsd socket, nothing on the LAN): a private cupsd 2.4.16 of this machine as the user on
127.0.0.1:8631 (own ServerRoot, spool, cache and logs in the session scratchpad) and tests/gnome/lib/fake_ipp.py as an
IPP Everywhere printer on 127.0.0.1:8632 (ippeveprinter refuses to start without Avahi). The request cups-pk-helper's
`PrinterAdd(name, uri, ppd, info, location)` builds (cph-cups.c: CUPS-Add-Modify-Printer, `ppd-name` in the
operation group, printer-name, device-uri, printer-info, printer-location) with `ppd-name everywhere` and
`device-uri ipp://127.0.0.1:8632/ipp/print`, sent with ipptool, answered `successful-ok (Local printer created.)`;
cupsd itself asked the printer (Get-Printer-Attributes with `all` and `media-col-database`, recorded by the fake) and
wrote `ppd/Eve.ppd` (124 lines, NickName "Fake LaserJet - IPP Everywhere", cupsFilter2 for pdf, urf and jpeg,
DefaultPageSize A4 from media-default). cupsd's binary has the error "IPP Everywhere driver requires an IPP
connection." for a non-IPP device URI. The new queue was **stopped and not accepting jobs**
(printer-state stopped, printer-is-accepting-jobs false), so:

* a driverless device (IPP 2.0 and image/pwg-raster, image/urf or application/pdf in its attributes or in DNS-SD's
  `pdl`) is added with `PrinterAdd(name, uri, "everywhere", info, location)`;
* a new queue is then `PrinterSetEnabled(name, true)`, `PrinterSetAcceptJobs(name, true, "")` and
  `PrinterAddOptionDefault(name, "media", ["a4"])` (g-c-c's pp-new-printer.c `printer_configure_async` and
  `printer_set_default_media_size`: "a4" outside the letter countries; nl and en-GB are A4);
* any other device gets "Stuurprogramma kiezen": the best matches of CUPS-Get-PPDs (ranked by the IEEE 1284
  MFG/MDL and the make-and-model, our replacement for GetBestDrivers), the whole list by manufacturer, or a PPD file
  (`PrinterAddWithPpdFile`); "Automatisch (IPP Everywhere)" is offered for every ipp/ipps/dnssd device.

Not verified: cups-pk-helper's own checks of the PPD name (it runs as root on the system bus with polkit, which the
sandbox cannot host); `everywhere` passes its printable-text check by its strings. SMB: only a typed `smb://` address
(libsmbclient is not loaded; amendment 41's second choice).

### Host

`host/src/services/printers.rs` (dispatch, argument checks, dry run, the watcher) with `printers/cups.rs` (libcups.so.2
through `dlib!`: one HTTP connection per call to the scheduler libcups chooses, a password callback that never asks on
a terminal), `printers/ipp.rs` (queues and jobs as JSON), `printers/ppd.rs` (GNOME's Printing Options from the PPD:
groups, PickOne/PickMany/Boolean, `*nl.Translation` and `*nl.Keyword` texts, `<hex>` bytes), `printers/discover.rs`
(DevicesGet, Avahi, typed addresses, driver ranking), `printers/helper.rs` (cups-pk-helper with interactive
authorization; NotPrivileged is `core.notAuthorized`) and `printers/tests.rs`. Colour: `devices/color.rs`, wired into
`devices.rs` (METHODS, `check`, `run`, the watcher). Methods (all in `METHODS` and `host/src/api-methods/devices.json`;
`!` dangerous, 🔑 secret):

| Method | | What it does |
|---|---|---|
| `printers.list {}` | | CUPS-Get-Printers (no temporary queues), CUPS-Get-Default or the user's `~/.cups/lpoptions` Default: `{available, reason?, default, printers: [{name, title, info, location, makeModel, state, reasons [{reason, severity}], accepting, deviceUri, markers [{name, level, colors, type}], commands, canClean, jobs, isDefault, ...}]}`; without cupsd `{available: false, reason: cupsd|libcups}` (GNOME's No Print Service) |
| `printers.get {name}` | | Get-Printer-Attributes `all` + the PPD (cupsGetPPD3, the temporary file removed): `{printer, ppd, options: [groups], ipp: [number-up, orientation-requested, sides when the PPD has no Duplex]}`, each option's value the queue's `<Option>-default`, else the PPD's |
| `printers.discover {address?, username?, password?}` | 🔑 | no address: Avahi `_ipp._tcp`/`_ipps._tcp` (3 s, resolved; ipps kept over ipp) beside `DevicesGet(10, 0, [], ["dnssd"])` (polkit devices-get, interactive); an address: a print server's shared queues (CUPS-Get-Printers), an IPP printer at `/ipp/print`, `/ipp/printer`, `/ipp`, JetDirect 9100 and LPD 515 (TCP probes), a typed URI as it is; `{devices, needsAuth?, denied?, helper?}` |
| `printers.drivers {deviceId?, makeModel?, driverless?}` | | CUPS-Get-PPDs `{drivers, best, everywhere}` |
| `printers.jobs {name?, which?}` | | Get-Jobs, every user's (my-jobs false) |
| `printers.testPage {name}` | ! | Print-Job of /usr/share/cups/data/testprint as the user, job-name `printers.testPageTitle` |
| `printers.cleanHeads {name}` | ! | when printer-commands has Clean: Print-Job `application/vnd.cups-command` "#CUPS-COMMAND\nClean all\n" (a memfd) |
| `printers.add {name, uri, ppd?, ppdFile?, info?, location?}` | ! | PrinterAdd / PrinterAddWithPpdFile; a new queue then enabled, accepting jobs and A4 (above); `{ok, name, new}` |
| `printers.rename`, `.setLocation`, `.setDefault`, `.setEnabled`, `.setAcceptJobs`, `.remove`, `.setOption` | ! | PrinterRename, PrinterSetLocation, PrinterSetDefault as g-c-c's printer_set_default (this computer's scheduler: the user's own `~/.cups/lpoptions` Default becomes a Dest line first, then PrinterSetDefault; another computer's scheduler: only the user's Default), PrinterSetEnabled, PrinterSetAcceptJobs(name, on, ""), PrinterDelete, PrinterAddOptionDefault |
| `printers.job.cancel`, `.job.hold`, `.job.release` | ! | JobCancelPurge(id, false), JobSetHoldUntil(id, "indefinite" / "no-hold") |
| `printers.job.priority {id, priority}` | ! | IPP Set-Job-Attributes job-priority as the user (GNOME's Move to Top) |
| `printers.jobs.clear {name}` | ! | every not-completed job JobCancelPurge on its own (pp-jobs-dialog.c): `{ok, canceled, failed}` when one was cancelled, else the first error |
| `printers.job.authenticate {id, username?, domain?, password}` | ! 🔑 | **added to the contract's list**: GNOME's authentication of a job held for it (CUPS-Authenticate-Job, auth-info in the order of the queue's auth-info-required) |
| `devices.color.get {}` | | colord's devices (displays first) with their profiles (the first is the default), every profile, the sensors |
| `devices.color.setEnabled`, `.makeDefault`, `.addProfile` ("hard"), `.removeProfile` | ! | Device.SetEnabled / MakeProfileDefault / AddProfile / RemoveProfile, `call_interactive` |
| `devices.color.installSystemWide {profile, device?}` | ! | Profile.InstallSystemWide (auth_admin_keep), then the device's default |
| `devices.color.importProfile {device, path}` | ! | the ICC (header and "acsp" checked) copied to `$XDG_DATA_HOME/icc` (same bytes: the same file; else "name-2.icc"), found by FindProfileByFilename (gsd's ICC store, 1.5 s) or CreateProfileWithFd(`icc-<md5>`, "temp", fd, {Filename, FILE_checksum}), then AddProfile |
| `devices.color.calibrate.start {device, quality, kind, whitepoint, title, sensor?}`, `.cancel`, `.resume` | ! | Mutter's `org.gnome.Mutter.ColorManager` CalibrateMonitor(the device's `XRANDR_name`) and the session's GammaLutSize first (review B), then colord-session ColorHelper.Display Start(device id, sensor id, {Quality, Whitepoint, Title, DeviceKind}) / Cancel / Resume; UpdateGamma's ramp goes to the session's SetCrtcGammaLut, Finished and a cancel Stop it |

Events: `printers.changed {}` (a CUPS notification subscription `dbus://`, lease 600 s renewed every 500 s on a
repeating source and cancelled by `shutdown` itself, as g-c-c's SUBSCRIPTION_DURATION / RENEW_INTERVAL; the first
org.cups.cupsd.Notifier signal of a burst arms a one-shot 250 ms emit; after every write),
`devices.changed {what: color}` (colord's Device/Profile/Sensor Added/Removed/Changed), `devices.calibrate {state:
sample|interaction|progress|gamma|finished, ...}` (ColorHelper.Display signals and its Progress). Stems:
`host/i18n/printers.tsv` (14 rows) and 6 more in `devices.tsv`, keys in `BASELINE-CHANGES.rust-settings-b`.
The Color and Printers panels write no GSettings key.

gio 0.18's `DBusConnection::call_with_unix_fd_list_sync` asserts on a reply without descriptors (colord's
CreateProfileWithFd answers none: the first host run panicked), so color.rs calls the C function without an out list.

### Native

* **printers** (printers.rs): Windows' page: "Een printer of scanner toevoegen · Apparaat toevoegen" (searches,
  lists the found devices under the card with their own "Apparaat toevoegen"; installed ones are left out; a
  driverless one is added at once, another after "Stuurprogramma kiezen"), "De printer die ik wil gebruiken, staat
  niet in de lijst · Handmatig toevoegen" (GNOME's address dialog with the print server's user and password when it
  asks); the queues as clickable cards ("Standaard", the status: Inactief, Bezig met afdrukken, Onderbroken or
  GNOME's reason such as Papier op); GNOME's printer search from five printers on; "Afdrukservice niet beschikbaar";
  Scanners › "Scanner openen" (apps.launch simple-scan.desktop, only with `caps.simpleScan`); related: Kleurprofiel,
  Apparaten. A new queue's name is g-c-c's canonicalized name ("HP-LaserJet-M404", "-2" when taken).
* **printer** (printers.rs, the data page `{name}`, titled by the printer; a queue removed or renamed elsewhere
  becomes "Deze printer is niet gevonden" on the next printers.changed, as GNOME's panel drops it): the hero with "Status: …", "Als standaard
  instellen", "Verwijderen" (Windows' question) and "⋯ Printkoppen reinigen" (only with Clean); the paused / rejecting
  notice with "Hervatten" (setEnabled + setAcceptJobs); Printerinstellingen: "Afdrukwachtrij openen" (the jobs with
  Onderbreken/Hervatten, Annuleren, ⋯ Bovenaan zetten, Verifiëren for a job held for authentication, "Alle documenten
  annuleren" with a question; it follows the scheduler every 2 s while open), "Testpagina afdrukken",
  "Printereigenschappen" (Naam, Locatie, Adres, Stuurprogramma, "Ander stuurprogramma kiezen"; a rename opens the
  page of the new name), "Voorkeursinstellingen voor afdrukken" (Indeling: pages per sheet, orientation, two-sided;
  then the PPD's groups; every change written at once, reverted when it fails); "Inkt en toner" (a bar per marker in
  its own colours); "Meer informatie" (model, location, address, queue name, shared).
* **colorprofile** (colorprofile.rs): an expander per device (GNOME's title, kind and default profile or "Niet
  gekalibreerd"; the first opens by itself): "Kleurbeheer gebruiken voor dit apparaat", "Kleurprofiel" (the
  dropdown), each profile with its date, "Voor alle gebruikers", colord's warnings and ⋯ (Instellen voor alle
  gebruikers with the shield glyph, Verwijderen), "Profiel toevoegen" (the compatible profiles and "Bestand
  importeren…"), "Kalibreren…" for a display while a sensor is connected (one dialog with GNOME's assistant pages, then
  a full-screen window with the sample colour, colord-session's questions with Hervatten, the progress, Annuleren and
  Gereed). "Sluit een kleursensor aan om een beeldscherm te kalibreren." without a sensor. No buttons on expander
  headers. Related: Beeldscherm, Printers en scanners.

Strings: `tests/i18n/windows-glossary/b.tsv` (slice 2 section; Windows' own where windows.json names one).

### plan.json controls dropped or changed

* printers "Printerlijst … Printers zoeken": Windows lists the printers without model, location and ink; the list
  shows Windows' status line, the model, location and ink are on the printer's page; GNOME's search shows from five
  printers on.
* printers: Windows' "Printervoorkeuren" (Windows mijn standaardprinter laten beheren, downloads over metered
  connections) and "Eigenschappen van afdrukserver" have no GNOME function (windows.json: omit); "Probleemoplosser
  uitvoeren" and "Hardware-eigenschappen" likewise.
* printer: GNOME's "Unlock" is not shown: Windows asks per action, and cups-pk-helper's polkit asks per call
  (YES for sudo and lpadmin users).
* printer "Voorkeursinstellingen": PickMany options show as one choice (CUPS PPDs hardly have them).
* printers.discover: SMB browsing is not there (amendment 41: no libsmbclient); a typed `smb://` address is taken as
  it is. LPD is probed as `lpd://host/` (the queue name is the user's to type in a full URI).
* colorprofile "Kalibreren…": review B added the gamma (it was not applied before): the host makes Mutter's
  calibration session (CalibrateMonitor(XRANDR_name) → session path, its GammaLutSize) before colord-session's Start,
  spreads every UpdateGamma ramp over the LUT as cc-color-calibrate.c's set_output_gamma does and sends it with
  SetCrtcGammaLut(aq, aq, aq), and Stops the session on Finished, a cancel, a new start and shutdown. The
  signatures are libmutter-18's and g-c-c 50's (strings of both binaries: `CalibrateMonitor (s) (o)`,
  `(@aq@aq@aq) SetCrtcGammaLut`, `GammaLutSize`, `XRANDR_name`, "gamma size is zero"); settings-devices.py checks
  them against the harness GNOME's own Mutter. A real calibration still needs a colorimeter (not testable here).
* colorprofile: GNOME's printer calibration and "View Details" need gnome-color-manager (not installed): left out
  as the plan says. Windows' "Kleur voor apps automatisch beheren" has no GNOME function (omit).

### Tests

* Host, `services::printers::tests` (16): the queue, job, driverless and IEEE 1284 parsers on fixtures; a PPD with
  groups, a value over two lines, Boolean and JCL options, nl and nl_NL translations, `<E9>`; DevicesGet's numbered
  keys; DNS-SD records; typed addresses; the driver ranking; lpoptions; the argument checks; libcups against an
  in-process IPP/HTTP responder on loopback (keep-alive, 100-continue, chunked; CUPS-Get-Printers, CUPS-Get-Default,
  Get-Printer-Attributes, the PPD over HTTP, Get-Jobs, CUPS-Get-PPDs, Print-Job with the test page and the clean
  command, Set-Job-Attributes, a closed port as No Print Service); every admin write against a cups-pk-helper fake on
  the private session bus (arguments and the interactive flag of each, a new driverless queue's four calls, a new
  driver for an existing queue only PrinterAddWithPpdFile, NotPrivileged, CUPS's refusal, DevicesGet, the typed
  address of the responder); the dry run calls neither the helper nor the scheduler. `services::devices::tests` (+6):
  colord objects, ICC checks and import names, the calibration's choices and events, every colour write against a
  colord + ColorHelper fake (paths, arguments, interactive, CreateProfileWithFd's id and properties), the colour dry
  run.
* Native: printers (queue names, the status, options and drivers, the search), colorprofile (titles, compatible
  profiles, the calibration choices): 7 new, Settings 37.
* Counts (16-11): host `--lib` 609 passed, 1 failed (`update::elf::tests::our_binaries_equal_readelf`, note 5, as on
  the base), 19 ignored, 27 warnings (the base's); native 72 passed, 1 ignored, no warning outside shell/ (the base's
  19); checks.sh as the base (rustfmt 0 failing, check_rust 0 errors / 57 warnings, drift 0 errors / 15 warnings
  after two allow rows in allow-host-settings-b.txt, check_native 48 uncovered, test_native OK; installer exit 70 as
  on the base).
* Harness: `settings-devices.py` steps 5-7 with fake_ipp.py (port 8631 on the sandbox's own loopback, the private
  `~/.cups/client.conf`) and the `cupspkhelper` and `colord` stand-ins of `standins/b.py`, `--source worktree`: the
  libcups host reads fake_ipp's queues (the stopped Epson with "Papier op"), "Apparaat toevoegen" (a real click) lists
  DevicesGet's two devices (DevicesGet interactive), adding the driverless one sends PrinterAdd('Fake-LaserJet',
  'ipp://127.0.0.1:8631/ipp/print', 'everywhere', 'Fake LaserJet', 'Lab'), PrinterSetEnabled, PrinterSetAcceptJobs and
  PrinterAddOptionDefault media ['a4'], all interactive; the printer's page (a real click on its card): Testpagina
  afdrukken reaches fake_ipp as a Print-Job "Testpagina" / "Test page" with the #PDF-BANNER test page, the queue
  lists jobs 41 and 42, pausing job 41 sends JobSetHoldUntil(41, 'indefinite') (by the dev channel's click, see note
  9), the preferences show PageSize A4 (the queue's PageSize-default over the PPD's), Duplex and ColorModel; the
  Epson page's "Hervatten" sends PrinterSetEnabled and PrinterSetAcceptJobs('Epson_ET_2850', true, ''), "Als standaard
  instellen" PrinterSetDefault; Kleurprofiel shows the display open with its EDID profile and the switch off sends
  Device.SetEnabled(false) interactive. The two known failures are note 9's (real clicks inside a kit dialog).

### Notes for I (slice 2)

6. `devices.json` lists `printers.job.authenticate` (dangerous, secret), a method §5.3 does not name: GNOME's
   authentication of held jobs. I adds it to the contract's list or drops it with its button.
7. The harness redaction (amendment 53, I's) must cover `printers.discover` and `printers.job.authenticate`.
8. VALIDATION_SETTINGS.md: amendment 41's result above (the IPP Everywhere queue and why `printers.add` enables it).
9. **Kit, every dialog (A's or I's, high priority)**: a real pointer click never reaches a button inside a
   `kit::controls::Dialog`, not even the dialog's own Sluiten (`w11-dialog-close`): the harness's real click on it
   (16-11 14:35, nl, `settings-devices.out` record `printers.dialogCloseClick`: pointer at the button's centre, the
   right window) left the dialog open, and the queue's pause button (`printers.holdClick`, the same coordinates as
   the screenshot shows the button) sent nothing, while the dev channel's click of the same button worked and real
   clicks on page buttons work. The cause, by GTK 4's gesture rules: `Dialog::show` puts a GestureClick on the smoke
   layer (`block`, bubble phase) that claims every press; GtkButton's own gesture claims only on release, so the
   smoke's claim on press cancels the button's sequence (the same mechanism as note 4's expander headers, where an
   ancestor button claims first). Keyboard (Enter, Escape, Tab) works, which is probably why it went unnoticed. Fix in
   `native/kit/src/controls/dialog.rs`: `block.set_propagation_phase(gtk::PropagationPhase::Target);` before
   `smoke.add_controller(block)`, so only a press on the dimmed area itself is claimed (the dialog's widgets are the
   targets of presses inside it; the page below is not in their propagation chain). Re-check with
   `settings-devices.py`'s `B-dialog-click` check (since the harness step one check that lists every dialog button
   the scenario pressed: Bestanden verzenden, Ja, the queue's pause and Sluiten), which then passes.

## Harness step: settings-devices.py in nl and en (16-11)

`tests/gnome/scenarios/settings-devices.py` now follows plan.json "testing" (GNOME HARNESS) for every page of the
batch, with the stand-ins of `tests/gnome/lib/standins/b.py` and `tests/gnome/lib/fake_ipp.py`. Run with `--source
worktree --strace <out>/strace.txt` (the Bluetooth page can reach rfkill and, through flight mode, nmcli; the guard
wants the strace file inside the output directory) on the release build of this branch (`build.sh b`, then again
after the page fix below).

### What it checks, per page

Every page: opened with `h.open('settings:<id>')` (the printer's data page by a real click on its card,
`h.settings_data_page`), a screenshot `<id>.<lang>` (more of a page: `<id>-<what>.<lang>`, 21 per run), the header
against registry.rs (a top-level page's title; else the `crumb-<id>` buttons of the parent chain and the current
crumb, a data page's being the printer's name), the current crumb not ellipsized, Settings in the run's language and
in en no word of `tests/i18n/dutch-words.txt` on the page (8 pages: all clean).

* **bluetooth, without BlueZ**: Windows' rows still listed (Apparaten, Printers en scanners, Muis, Automatisch
  afspelen), no switch; USB listed and found by the search exactly when `caps.thunderbolt` (false here: no
  `/sys/bus/thunderbolt`, so USB is hidden and the search for "usb" finds Automatisch afspelen and Opslag only).
* **autoplay**, every bound control by its `st-autoplay-*` name with real pointer clicks and GNOME's exact value in
  the run's keyfile: the switch is the inverse of autorun-never (Ubuntu's schema default is true; the scenario reads
  defaults from the installed schema files), its click writes it; Verwisselbaar station › Geen actie ondernemen writes
  `automount-open=false`; the five media rows (Geheugenkaart › Map openen, Audio-cd › Geen actie, Dvd-video › Geen
  actie, Muziekspeler › Map openen, Software › Geen actie) each write GNOME's three lists in one go, ending with
  start-app `['x-content/ostree-repository']`, ignore `[audio-cdda, video-dvd, unix-software]`, open-folder
  `[image-dcf, audio-player]` (the record `autoplay.lists` holds these final lists); outside writes of autorun-never
  (both ways), automount-open and the ignore list move the switch and drop-downs; with AutoPlay off the rows are
  insensitive; Andere mediatypen opens GNOME's dialog with Type and Actie. No key of `allow/devices.rs` is Owned, so
  no row points to a WinPoint page; a key lock cannot be made in the sandbox (README: no `/etc/glib-2.0`).
* **bluetooth, with BlueZ and gsd's Rfkill** (new stand-ins): the switch on, "Detecteerbaar als “WinPoint-pc”"
  (BlueZ's Alias), `Set Discoverable true` while the page shows and `false` once it goes (the lease, only the host's
  own); the three paired devices as tiles, the mouse with BlueZ's 70 %; the switch off sends gsd's
  `BluetoothAirplaneMode true` first (amendment 57), then, because the stand-in adapter does not follow gsd, BlueZ's
  `Powered false` from the host's fallback, and shows "Bluetooth staat uit"; on again: `BluetoothAirplaneMode false`
  and `Powered true`; with gsd's flight mode on, GNOME's "Vliegtuigmodus staat aan" notice with its button.
* **bluetooth-devices** (BlueZ and UPower): Windows' groups (Bluetooth mouse with 70 % and the receiver mouse with
  UPower's 55 % under Muis, toetsenbord en pen; headphones under Audio; the phone under Overige apparaten), not the
  laptop battery; ⋯ › Verbinding verbreken sends `Device1.Disconnect` to the headphones; ⋯ › Eigenschappen opens
  GNOME's device dialog (Gekoppeld, Type, Adres), whose Bestanden verzenden runs `bluetooth-sendto
  --device=C4:DD:57:AA:BB:02 --name=Pixel 8` (the stub's argv in `logs/guard-calls.log`); ⋯ › Apparaat verwijderen
  asks Windows' question and Ja sends `Adapter1.RemoveDevice(<the phone's path>)`, after which the card goes.
* **usb** (bolt, by its deep link): as before (AuthMode, the pending dock, `EnrollDevice(uid, 'default', '')` and
  `Set AuthMode 'disabled'`, the enrolment interactive).
* **printers / printer** (fake_ipp.py, cups-pk-helper): as before (both queues and GNOME's reason, DevicesGet and
  amendment 41's four calls, all interactive; the test page, the queue's `JobSetHoldUntil(41, 'indefinite')`
  interactive, the PPD's options, Hervatten, `PrinterSetDefault` interactive), plus the header of both printers'
  pages and Scanner openen (below).
* **colorprofile**: as before, plus the Kleurprofiel drop-down (a real click) sending
  `Device.MakeProfileDefault('…/icc_calibrated_dell')` with interactive authorization; without a colour sensor no
  Kalibreren… but the notice, with the `colord-sensor` stand-in Kalibreren… and no notice (plan step 8).

Polkit: every write that goes through polkit reaches its stand-in with ALLOW_INTERACTIVE_AUTHORIZATION (bolt,
cups-pk-helper, colord). Properties.Set (bolt's AuthMode, BlueZ, gsd) is answered by GDBus itself, so the stand-ins
cannot see its flag; the host unit tests check it. strace: connect and exec checks OK in both runs (no pkexec,
systemctl, nmcli or other forbidden program ran; WinPoint started no Python).

### Stand-ins added to `standins/b.py`

* `bluez` through `start_objects` (an own server in the same file: the frozen builder answers only plain values,
  and GetManagedObjects is `a{oa{sa{sv}}}`): ObjectManager on `/`, Adapter1 hci0, three Device1 (+ Battery1),
  AgentManager1; Set of Powered/Discoverable, Connect, Disconnect and RemoveDevice change the objects and emit
  PropertiesChanged / InterfacesRemoved. Tested first on a throwaway dbus-daemon inside bwrap.
* `gsd-rfkill` and `gsd-rfkill-flight` (org.gnome.SettingsDaemon.Rfkill, session bus), `colord-sensor`; colord's
  display now has both profiles (the drop-down needs two) and the calibrated one a language-neutral title.

### Fixed in the pages

* `bt_devices.rs`: a UPower-only device showed its battery twice ("Batterij 55%" in the status line and "55%" as
  the card's control, seen in `bluetooth-devices.nl.png`); now only the status line, as Windows and the Bluetooth
  cards show it.

### Runs (all `--source worktree`, with `--strace`)

| Run | Checks passed | Failed |
|---|---|---|
| nl 15:33 (first) | 97 / 101 | 4, 1 known: the flight-mode notice (the scenario compared a markup label exactly: fixed), Bestanden verzenden (note 10), Scanner openen (diagnosed in the next run: note 12), B-dialog-click (known) |
| nl 15:46 (page fix built) | 104 / 109 | 5, 4 known; Scanner openen's backend probe gave SUN_LEN, then marked B-sun-len |
| en 15:50 | 113 / 117 | 4 known: B-stubs-umask, B-sun-len, B-crumb-ellipsis (HP's page; Epson's fits in en), B-dialog-click |
| nl 16:02 (final, committed) | 104 / 109 | 5 known: B-stubs-umask, B-sun-len, B-crumb-ellipsis on both printers' pages, B-dialog-click |

Regression on this branch's final build (nl, `--source worktree`): `native-shell.py` 21 / 21 (16:21) and
`lock-unlock.py` 5 / 5 (16:35), the same as their committed results; their output went to untracked directories
(those result folders are not batch B's files) and was removed afterwards. Native tests after the page fix: 72
passed, 1 ignored (Settings 37), no warning outside shell/.

Not covered by the harness (host unit tests or nothing): pairing in Een apparaat toevoegen (an agent, BlueZ's
pairing replies), the calibration itself and colord-session, Avahi discovery, the driver chooser, the manual
address dialog and the other dialog-only printer writes (clicks inside dialogs, note 9), the choices inside Andere
mediatypen (the same), Thunderbolt caps true (sysfs is the host's; `services::caps` unit tests cover the fixture),
key locks.

### Notes for I (harness step)

10. **Harness, tests/gnome/harness.py (frozen)**: `prepare()` makes `run/stubs` with `mkdir` and the umask's mode;
    under Ubuntu's umask 0002 that is 0775, and `host/src/tools.rs` `private_dir` refuses a group-writable seam
    directory (by design, no fallback), so in the harness **every** `tools::system_tool` spawn fails (all batches:
    bluetooth-sendto, grdctl, tecla, passwd, localedef, ubuntu-insights, simple-scan through system_tool, pkexec
    helpers). settings-devices.py records it (`B-stubs-umask`, `0o775`) and then chmods the directory 0755 itself.
    Patch, after the `for sub in (...)` mkdir loop in `prepare()`: `os.chmod(RUN / 'stubs', 0o755)`.
11. **Kit Breadcrumb (`native/kit/src/controls/misc.rs`, I's)**: with a long chain the current crumb is the one that
    shrinks: the printer's page shows "Bluetooth en apparaten › Printers en scanners › …" (nl, 36 px) and "… › HP L…"
    (en). Windows' BreadcrumbBar collapses the leading items into "…" and keeps the current page. Proposed: the item
    buttons get an ellipsizing label (`Label::new`, `set_ellipsize(End)`, `set_width_chars(3)`) and the current label
    keeps its natural width up to about 24 characters (`set_width_chars(min(chars, 24))`), so the earlier items give
    way first. `B-crumb-ellipsis` in settings-devices.py then passes.
12. **files/actions.rs `launch_gio` (not a batch file)**: the launcher's ack socket is bound in `$TMPDIR`
    (`win11-launch-<pid>-<nonce>/ack`); in the harness copy `run/tmp` is 152 characters, so the socket path is longer
    than SUN_LEN and every `apps.launch` fails there ("path must be shorter than SUN_LEN": the scanner probe in
    `results.json`). A real session's /tmp is short, but a long TMPDIR breaks every app launch the same way. Fix:
    bind it in `$XDG_RUNTIME_DIR` when set (short and private), else `temp_dir()`.

## Review B (16-11): the eleven findings

Each finding was first checked against the code; all eleven held. Per finding: what was wrong, the fix, the test.

| # | Finding | Verdict | Fix | Test |
|---|---|---|---|---|
| 1 | The discoverable lease outlived a closed window | fixed | bluetooth.rs `Lease`: the window's close-request sends `{on: false}` on a thread and waits for it (at most 3 s; the app and its PDEATHSIG backend end right after), no renewal after; the cleanup disconnects the handler. The host side (a SIGTERM'd backend never reaches `Devices::shutdown`) is note 13 for I. Docs of devices.rs, devices/bluetooth.rs, bluetooth.rs and devices.json corrected | settings-devices.py `window_closes_on_bluetooth`: the Bluetooth page again, `h.close('settings')` (a user's window close), then Set Discoverable false |
| 2 | Both watchers woke the CPU while idle | fixed | devices: no tick; the first signal of a quiet kind arms a one-shot 250 ms emit; the lease's end is a one-shot source (re-armed after a renewal, its state under the lease's lock). printers: `iteration(true)`, a one-shot emit per burst, a repeating renewal source | host `the_watcher_sleeps_and_emits_a_burst_of_signals_once` (five UPower signals, one event, nothing after), `a_lease_without_a_renewal_ends_by_itself` (600 ms lease, the renewal pushes it on, shutdown ends one), `the_watcher_holds_gnomes_subscription_and_sleeps_between_bursts` |
| 3 | CUPS subscription lease 3600 s, not cancelled at a real exit | fixed | lease 600 s, renewal every 500 s (g-c-c's SUBSCRIPTION_DURATION / RENEW_INTERVAL); `Printers::shutdown` takes the id from a shared lock and cancels at once (a renewal under way finishes first, then sees `stopped`). Found on the way: `cups::RENEW_SUBSCRIPTION` was 0x0018, which is Get-Subscription-Attributes, so the lease was never renewed at all; now 0x001A (libcups' op table: Create-Printer-Subscriptions 0x16 … Renew-Subscription 0x1A, Cancel-Subscription 0x1B) | the watcher test above: Create (lease 600, `dbus://`, the events), a Renew-Subscription with id 7 and lease 600, Cancel with id 7 inside `shutdown()`, no renewal afterwards |
| 4 | setDefault left a per-user default pin | fixed | g-c-c's printer_set_default: `cups::is_local(cupsServer())` (localhost, 127.0.0.1, ::1 prefix or a socket): `lpoptions_without_default` (Default → Dest, no Default added) before PrinterSetDefault; another computer's scheduler: only `lpoptions_with_default` (a missing `~/.cups` made 0700), no helper call | `lpoptions_follows_gnomes_printer_set_default`; the admin test expects `Dest Epson\nDest HP_LaserJet sides=one-sided\n` and, for `print.example.invalid`, a new lpoptions `Default Kantoor` without PrinterSetDefault |
| 5 | decode_hex panicked on an odd digit count | fixed | spaces filtered first, then non-empty, even and all hex, else '<' stays literal | `odd_hex_in_a_ppd_stays_text` (`<1 >`, `<E9 2>`, `<>`, `<E`, and a PPD with them parses) |
| 6 | Clear All stopped at the first refusal | fixed | every job on its own, `clear_answer`: `{ok, canceled, failed}`, the first error only when none was cancelled | `clear_all_cancels_every_job_on_its_own`; the admin test: the fake refuses job 41 (NotPrivileged), job 42 is still cancelled, `{canceled: 1, failed: 1}` |
| 7 | Verwisselbaar station wrote a key nothing reads | fixed (kept as Windows' row) | the row shows GNOME's fixed "Altijd vragen" (value label, `st-autoplay-removable`), greyed with the rows; automount-open out of `allow/devices.rs`; note and scenario corrected | native `a_removable_drive_is_always_asked_about`; settings-devices.py: the fixed value, no `st-autoplay-automount-open`, `gsettings.set` of automount-open refused and the keyfile unchanged, insensitive with AutoPlay off |
| 8 | 15 page strings in no glossary file | fixed | the scan below found 17 (also "Tablet", "Bluetooth & devices"); all in b.tsv (winpoint/no, "Bluetooth & devices" glossary/yes), plus "Depends on" | the scan: 518 pairs, 0 missing; test_native.py OK |
| 9 | The printer page kept a printer that went elsewhere | fixed | `PrinterPage::reload` uses `p.call`: `not_found` (printer_gone) shows `printer_missing()` ("Deze printer is niet gevonden", the same as the builder's), other errors keep the page | native `a_queue_that_went_elsewhere_is_not_found` |
| 10 | The calibration's UpdateGamma was dropped | fixed (implemented) | as GNOME 50 (strings of /usr/bin/gnome-control-center: `XRANDR_name`, `org.gnome.Mutter.ColorManager`, `CalibrateMonitor (s) (o)`, `(@aq@aq@aq) SetCrtcGammaLut`, `GammaLutSize`, "gamma size is zero", "no data in the CLUT array"; libmutter-18 the same interface and `/org/gnome/Mutter/ColorManager/Calibration/u%u`): CalibrateMonitor before Start, the ramp spread over GammaLutSize points as set_output_gamma (integer floor/ceil, no libm), SetCrtcGammaLut, Stop on Finished / cancel / new start / shutdown; no Mutter or no connector: `unsupported devices.noCalibration`, nothing started | host `colour_writes_go_to_colord_with_the_password_dialog` (CalibrateMonitor('DP-1') before Start, the LUT values, Stop), `a_gamma_ramp_is_spread_over_the_lut_as_gnome_does`, `no_mutter_colour_manager_is_no_calibration`; settings-devices.py `mutter_calibration` against the harness GNOME's own Mutter |
| 11 | No "Depends on" before authorising a chain | fixed | `depends_on` (chain minus the device, names or "Thunderbolt-apparaat"): "Afhankelijk van" card before "Autoriseren en verbinden" | native `a_device_names_the_unauthorised_devices_it_depends_on` |

The glossary scan (finding 8), for I's check: every `t("nl", "en")` pair of bluetooth.rs, bt_devices.rs,
printers.rs, autoplay.rs, usb.rs and colorprofile.rs against the (en, nl) rows of every
`tests/i18n/windows-glossary/*.tsv`, pairs that were in native/ at af8eebdc exempt (amendment 28: existing strings
stay). The script was a throwaway in the session scratchpad; its regex is
`\bt\(\s*"((?:[^"\\]|\\.)*)"\s*,\s*"((?:[^"\\]|\\.)*)"\s*,?\s*\)` (multi-line, `\"` unescaped).

### Tests and runs after the review (16-11)

* Host `--lib` (full, tests/rust-sandbox): 616 passed, 1 failed (`update::elf::tests::our_binaries_equal_readelf`,
  note 5, as on the base), 19 ignored (609 before: +7 new, `lpoptions_keeps_the_users_default_in_step` became
  `lpoptions_follows_gnomes_printer_set_default`); the batch's filter (`services::devices services::printers
  services::gsettings services::system::bluetooth`) 57 passed. No warning in batch B's files.
* Native (`cargo test --release`): 75 passed, 1 ignored (Settings 40: +3), no warning outside shell/.
* checks.sh: as the base (rustfmt 0 failing, check_rust 0 errors / 57 warnings, drift 0 errors / 15 warnings,
  test_native OK, release check OK; installer exit 70 as on the base). The glossary scan: 518 pairs, 0 missing.
* settings-devices.py (`--source worktree --strace`, release build of this branch):

| Run | Checks passed | Failed |
|---|---|---|
| nl 18:40 (first) | 92 / 113 | 21, 15 not known: the Bluetooth switch's first real click never reached gsd's Rfkill within 35 s (the page waited for `system.bluetooth.setPowered`; the Set came after the scenario's 10 s), so the switch-off checks failed and the adapter stayed half off for Apparaten, its ⋯ menus and the window-close check (no lease asked with Bluetooth off). Not reproduced: en and the nl rerun pass these in 0.5 s, as the 16:02 run did |
| en 18:55 | 117 / 122 | 5 known: B-stubs-umask, B-sun-len, B-crumb-ellipsis (HP), B-mutter-virtual (new), B-dialog-click |
| nl 19:07 (committed) | 108 / 114 | 6 known: B-stubs-umask, B-sun-len, B-crumb-ellipsis on both printers, B-mutter-virtual, B-dialog-click |

  New checks pass in both languages: Verwisselbaar station's fixed "Altijd vragen", no automount-open control,
  gsettings.set refusing automount-open, the row greyed with AutoPlay off; "closing the window on the Bluetooth page
  ends the lease" (Set Discoverable false 0.5 s after `h.close('settings')`); Mutter's CalibrateMonitor is `(s) →
  (o)`. `B-mutter-virtual`: the harness's virtual monitor Meta-0 answers CalibrateMonitor with "Device cannot be
  calibrated (0)" (it has no gamma LUT); that answer also shows the argument is the connector (an unknown one is
  "Unknown monitor"). The session's own signatures (GammaLutSize, SetCrtcGammaLut(aq, aq, aq), Stop) therefore rest
  on the two binaries' strings and g-c-c's `(@aq@aq@aq)`; the host reads GammaLutSize as any integer type.

### Notes for I (review)

13. **host/src/lib.rs `run_http_only` (frozen)**: the http-only backend has no SIGTERM handler, so a Settings backend
    ended by PR_SET_PDEATHSIG (the normal end, and a crashed Settings) dies without `runtime.shutdown()`: no
    `Devices::shutdown` (the discoverable lease, Mutter's calibration session) and no `Printers::shutdown` (the CUPS
    subscription; with this review its lease ends by itself within 10 minutes). Patch, just before `main_loop.run();`:

    ```rust
    // SIGTERM (PR_SET_PDEATHSIG when the app ends, a crashed app, kill): end as a quit does, so runtime.shutdown()
    // runs the services' shutdown (devices: the discoverable lease, the calibration; printers: the subscription)
    {
        let main_loop = main_loop.clone();
        glib::unix_signal_add_local(libc::SIGTERM, move || {
            main_loop.quit();
            glib::ControlFlow::Break
        });
    }
    ```

    (glib 0.18 `unix_signal_add_local(signum, FnMut() -> ControlFlow)`; the handler is one-shot, so a second SIGTERM
    during a hanging shutdown kills as before). It changes every http-only host (panel, explorer, the apps): I checks
    that their shutdown stays short. Test: the Bluetooth page open, `kill -TERM` of the Settings backend in the
    harness, then the BlueZ stand-in's Set Discoverable false.

## CHANGELOG lines for I (batch B)

For the next release's section of CHANGELOG.md (I's file), in its style:

- **Bluetooth & devices as on Windows, with all of GNOME's settings.** Settings › Bluetooth & devices now lists
  Devices, Printers & scanners, Mouse, Touchpad, Pen, AutoPlay and USB under the Bluetooth card, also on a computer
  without Bluetooth. The PC is discoverable under its own name while the page is open, and no longer once you leave
  the page or close Settings. Flight mode shows GNOME's notice with **Turn off flight mode**.
- **Devices.** Mice, keyboards and pens, audio and other devices in Windows' groups, each with its battery (also a
  receiver mouse's battery from UPower), **Connect** / **Disconnect**, **Remove device** and **Properties** with
  GNOME's details and **Send files** for phones and computers.
- **AutoPlay.** Turn AutoPlay on or off and choose what happens for memory cards, audio CDs, DVDs, music players,
  software and other media, as in GNOME's Removable Media settings.
- **USB.** On a computer with USB4 or Thunderbolt: **Direct access**, the connected docks and devices with their
  status, **Authorise and connect** (also for the dock a device depends on, which the page names) and **Forget
  device**, each asking for the administrator password.
- **Printers & scanners.** Add a printer that's found on the network or by its address; a driverless printer is
  ready at once. Each printer has its own page: set it as default, print a test page, the print queue (pause, resume,
  cancel, move to the top, cancel all), printing preferences from its driver, ink and toner, clean the print heads,
  rename it or change its driver, and remove it. **Open scanner** starts Document Scanner when it's installed.
- **Colour profile.** Every display, printer and camera colord knows, with its default profile, adding, importing and
  removing profiles, **Set for all users**, and calibrating a display with a colour sensor, as in GNOME's Color
  settings.
