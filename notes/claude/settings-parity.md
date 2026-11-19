# Settings parity: the contract for batches B0, A-F and I

Robin, 16-11: "alle settings van de originele GNOME functies moeten erin komen, maar in de style van windows settings
app. kijk vooral af". WinPoint's Settings (native/apps/settings, GTK 4 + Rust) gets every function of GNOME Settings
50 (gnome-control-center 50.3) in the structure and wording of Windows 11 Settings (25H2, nl-NL / en-GB).

Sources, committed next to this file in `notes/claude/settings-parity/`: `plan.json` (90 pages with their 371
controls, each with its GNOME function, backing keys and writes; the backend design; batches B0, A-F, I;
`testing`), `critique.md` (the 69 numbered amendments, in full) and the four inventories `winpoint.json`,
`gnome-a.json`, `gnome-b.json` (GNOME Settings 50's functions and keys) and `windows.json` (Windows 11's pages and
strings). A batch reads its pages' controls in plan.json and its amendments in section 1 below (critique.md has
their reasoning). The inventories still name the session scratchpad where their authors kept extracted .ui files;
those copies are gone, the facts are in the JSON. This file is the FINAL contract: where it differs from the plan,
this file wins. A batch that finds this contract wrong writes it in its handoff note; it does not change the
contract (or another batch's files) on its own.

Batch B0 (this commit range) builds the foundation. Batches A-F then run in parallel worktrees, each branched from
B0's last commit, and only edit the files they own (section 4). Step I merges them, applies the handoff notes and
removes the placeholder.

Contents: 1 Decisions on the critique. 2 The page tree. 3 Shared-file rules. 4 Ownership per batch (with the
multi-monitor worktree). 5 Host API per batch (method names, arguments, events, stems, allowlist file). 6 The
foundation API (B0). 7 Tests. 8 Integration notes (what B0 could not do because another worktree owns the file).

## 1. Decisions on the critique amendments

"Apply" means the amendment is part of this contract (and of the named batch's work); "Reject" gives the reason.

| # | Decision |
|---|----------|
| 1 | Apply. New page `sound-devices` (Alle geluidsapparaten / All sound devices), parent `sound`, module `sound_device.rs` (D). The alias `sound-devices` now names this page. |
| 2 | Apply (D): "Een nieuw invoerapparaat koppelen · Apparaat toevoegen" → `p.go_with("bluetooth", {"add": 1})`. |
| 3 | Apply (D): "Opstartgeluid afspelen" moves into the "Meer geluidsinstellingen" dialog, tab Geluiden. |
| 4 | Apply (F): the WinPoint toast path calls `notification.playSound()` in `extension/notificationsBridge.js` unless `notifications.sounds` is false or DND is on; toast and list honour `policy.enable` and `policy.forceExpanded`. The plan's "drop the sound before GNOME plays it" step is deleted. Test: amendment 63. |
| 5 | Apply (F): lock-screen and sound rows inside the "Meldingen" expander. |
| 6 | Apply (F): `notifications.app.set` writes `application-children` plus the relocatable child (with `application-id`) as g-c-c does. In the allowlist `application-children` is `Read` and the per-app keys are `Service("notifications.app.set")`. |
| 7 | Apply (F): PermissionStore id `gnome`/`shortcuts-inhibitor`; Autostart row enabled only while Background is allowed; empty state "Zoeken in App Center". |
| 8 | Apply (A): "Aanraakscherm toewijzen aan beeldscherm" when `caps.touchscreen`; the relocatable touchscreen schema is in `allow/input.rs` (B0 put it there). |
| 9 | Apply (B0 + A): path holes `{serial}` = `[0-9a-f]{1,16}` or `default-<vid>:<pid>`, `{padctl}` = `(button|ring|strip|dial)[A-Z](-(cw|ccw|up|down))?` (libmutter-18: `stylus/%lx/`, `stylus/default-%.4x:%.4x/`, `tablets/%.4x:%.4x/%s%c%s/`). |
| 10 | Apply (A): no sensitivity dropdown; `disable-while-typing-timeout` is in no allow table (B0 test). |
| 11 | Apply (D): the hot-corner row is labelled "Geldt in de GNOME- en Ubuntu-sessie" / "Applies in the GNOME and Ubuntu sessions" and placed last as a WinPoint addition. Making the corner open Task View would need `desktopBridge.js` (multi-monitor worktree): rejected for now. |
| 12 | Apply (D): Windows' two rows in the Bureaubladen expander. "Op de taakbalk alle geopende vensters weergeven" = setting `taskbar.allDesktops` (B0 adds the default; its consumer is in `native/shell/src/panel/**`, section 8). "Alle geopende vensters weergeven wanneer ik op Alt+Tab druk" = setting `taskView.allDesktops` (consumer `native/shell/src/flyouts/menus/taskview.rs`, D) and the same value written to `org.gnome.shell.app-switcher` and `org.gnome.shell.window-switcher` `current-workspace-only` (inverted). Order: Vensters uitlijnen, Bureaubladen, then the WinPoint additions. |
| 13 | Apply (A + B0): the extension's `task-manager` and `task-view` are editable through `input.shortcuts.set`. B0's held check is per accelerator (`gsettings/owned.rs` `held_accel`): `<Alt>Tab` in any `org.gnome.desktop.wm.keybindings` key while the extension's `task-view` holds it, Ctrl+Alt+Delete in media-keys `logout` while `task-manager` holds it. `gsettings.set` refuses such a value with `denied gsettings.heldByWinPoint` (data: accel, setting); A shows it as "In gebruik door Taakweergave" / "In gebruik door Taakbeheer". |
| 14 | Apply (A): the note on `a11y-keyboard` ("Met Plaktoetsen opent de Windows-toets Start niet" / "With Sticky Keys on, the Windows key doesn't open Start"). |
| 15 | Apply, deferred: the tray input indicator lives in `native/shell/src/panel/taskbar/input.rs`, which is in the multi-monitor worktree's tree. A writes it as a handoff patch; I applies it after the multi-monitor merge (section 8). |
| 16 | Apply (E): 24-hour and first day only on `formats`, with a link from `datetime`; `datetime` in Windows order. |
| 17 | Apply (E): the regional format inline on `language` (dropdown + expander with samples + "Notaties wijzigen" → `formats`). |
| 18 | Apply (E): one "Weergavetaal" row (AccountsService `SetLanguage`, sign-out banner) and a "Taal van WinPoint" override row. The `language` setting gains `"system"`. E is the only batch that edits `host/src/settings.rs` (`valid_language`), the language resolution in `host/src/i18n.rs` (`language_in` and its parse, not STEMS) and `native/kit/src/i18n.rs`. |
| 19 | Apply (D), Display layout: the presentation-mode dropdown (Dupliceren / Uitbreiden / Alleen weergeven op N) beside Identificeren in the arrangement card; the "Meerdere beeldschermen" expander closes "Schaal en indeling" and holds "Dit mijn hoofdbeeldscherm maken" and "Ander beeldscherm detecteren"; Kleurprofiel is a row in "Helderheid en kleur" (24H2), not a related link; Related settings: Geavanceerd beeldscherm. |
| 20 | Apply (D), Power & battery layout: the power-button dropdown inside the expander "Besturingselementen voor deksel en aan/uit-knop", one column because GNOME has one value, no lid rows; both energy-saver rows inside the "Energiebesparing" expander; "Batterij opladen" and "Batterijniveaus" in the Batterij section; GNOME's warning when automatic suspend is set to Nooit. |
| 21 | Apply (D): presets Nooit, 1 dag, 14, 30, 60 dagen; a non-preset current value is an extra option; "1 uur" (0) is a GNOME extra; the master switch sets both keys; "Opslagzin nu uitvoeren". |
| 22 | Apply (F), Default apps as Windows 11 (no category dropdowns at the top): the search "Een standaardwaarde instellen voor een bestandstype of koppelingstype"; the list "Standaardwaarden instellen voor toepassingen", whose app page shows GNOME's Files & Links (the builder of `app-advanced`'s handlers section); "Alle standaard-apps opnieuw instellen". |
| 23 | Apply (E): "Als je weg bent geweest, wanneer moet WinPoint vereisen dat je je opnieuw aanmeldt?" under "Aanvullende instellingen". |
| 24 | Apply (A), Accessibility › Keyboard: Windows shows "Sneltoets voor Plaktoetsen / Filtertoetsen / Wisseltoetsen" inside each expander; GNOME has one key (`a11y.keyboard enable`), so the row is in all three expanders, bound to that key, with a note that it covers all three; no separate row. |
| 25 | Apply (D): "Nu uitschakelen" sets DisabledUntilTomorrow; "Nu inschakelen" only while it is true; outside the schedule an info line ("Wordt om 20:00 ingeschakeld"). The schedule is never rewritten. |
| 26 | Apply (B0): `registry::DATA_PAGES` = wifi-network, printer, app-advanced, app-notifications, sound-device. Excluded from search, Start's `settings_pages.json`, Home's recent list and aliases. |
| 27 | Apply: B0 adds the line on Bureaubladpictogrammen (`desktopicons.rs`); the Taakbalk line is in `pages/taskbar.rs` (multi-monitor worktree): section 8. |
| 28 | Apply (B0 seeds): `tests/i18n/windows-glossary/<batch>.tsv` (columns en, nl, source, verified), one file per batch so parallel batches never touch the same file; batches take their UI strings from it; I adds the check that fails on new UI strings missing from all of them. Existing strings stay until a verified Windows string replaces them. |
| 29 | Apply (B0): the deliberate renames are rows of `windows-glossary/b0.tsv` (source "rename"). |
| 30 | Apply (E): no timesync1; `machine.time.sync` = timedate1 `SetNTP(false)` then `SetNTP(true)`; status from `NTPSynchronized`; the server from chrony's sources (read-only) or the row is hidden. |
| 31 | Apply (D): `system.brightness.*` through DisplayConfig `SetBacklight`/`Backlight`, logind kept for Hyprland and as fallback. |
| 32 | Apply (D): `displayBridge.js` blocks the shell's `confirm-display-change` handler while a consumer is attached and relays `display.confirmChange`; Settings answers through `display.confirm`. B0 relays `display.*` bridge events (`host/src/gnome.rs`). The dispatch line in `panelBridge.js` is an integration note. |
| 33 | Apply (D): fixtures, not an interface file. |
| 34 | Apply (C): `libgnutls.so.30` joins the dlopen list. |
| 35 | Apply (C): `ssh.socket` when present; expect a password prompt. |
| 36 | Apply (A): Orca preferences through D-Bus `org.gnome.Orca.Service` `ShowPreferences`; `launchBridge.js` stays for IBus engine setup and the Security Center. |
| 37 | Apply (E): `org.gnome.Shell` `ShellVersion`. |
| 38 | Apply (F): `/usr/bin/ubuntu-insights` through `tools::system_tool`; no libinsights. |
| 39 | Apply (D): `libpulse-mainloop-glib.so.0` on the sound service's own MainContext; `system.audio.*` delegates to `sound.*` when libpulse loads (D owns `host/src/services/system/audio.rs` and `audio_monitor.rs`); one `sound.changed` event. |
| 40 | Apply (C): spike libnma (`nma_ws_*`, the GTK 4 VPN editors) before hand-writing forms; record the outcome. |
| 41 | Apply (B), Printers: verify before B starts how a driverless (IPP Everywhere) queue is created through cups-pk-helper (g-c-c relies on the Python `GetBestDrivers` service the plan drops); `cupspkhelper.*` is YES for sudo/lpadmin users (`com.ubuntu.desktop.rules`), otherwise auth_admin_keep; SMB browsing needs libsmbclient (which g-c-c links): add it to the dlopen list, or support only a typed `smb://` address. |
| 42 | Apply (F): `X-Allow-Interaction: true`. |
| 43 | Apply (F): bus name `org.freedesktop.impl.portal.desktop.gnome`, interface `org.gnome.GlobalShortcutsRebind`, path `/org/gnome/globalshortcuts` (xdg-desktop-portal-gnome); `global-shortcuts applications` is read natively, it is in no allow table. |
| 44 | Apply: `pickers::KeyCapture` (B0) asks the host first (`input.capture`, which A implements through `inputBridge.js`: `Main.pushModal`, one key event, release) and falls back to a local GTK key controller while the method is unknown. No inhibit prompt, no PermissionStore write. |
| 45 | Apply (F): `index.setRoots` first, the same folders mirrored into `org.freedesktop.Tracker3.Miner.Files`; the search-providers page says it is GNOME's overview search. |
| 46 | Apply (A) through the B0 hook: `settings_effects.rs` calls `settings_mirror::render::effects(..)` (A's file) on every change in the panel host. |
| 47 | Apply (B0), refined by the B0 review (finding 4): at every panel start the side that moved since the values both last agreed on (`<config>/gnome-mirror.json`, section 6.4) wins, GNOME when both moved or when there is no record yet (the first start: GNOME → settings.json for every preference pair, as the amendment says); the accent look pair goes settings.json → GNOME only. So a settings.json change made by another host while the panel was down reaches GNOME instead of being reverted. CHANGELOG note by I. |
| 48 | Apply: `time.firstDayOfWeek` uses `-1` for "Standaard (regio)" (B0, default now -1). The textScale range is A's (render pairs). |
| 49 | Apply: Settings pages write the GNOME key (allow mode `Mirrored(setting)`), the mirror updates settings.json; other WinPoint UIs keep writing settings.json and the mirror writes GNOME. The textScale, reduceMotion and highContrast pairs ship with A in `host/src/settings_mirror/render.rs` (empty in B0), so main never double-scales. |
| 50 | Apply (B0, with a test): every (schema, key) in exactly one allow table. Assignments as the critique lists; `allow-volume-above-100-percent` is in `system.rs` (D). |
| 51 | Apply (B0): `gsettings.set` also takes `{writes: [{schema, path?, values}, ...]}`, validates all of them first and writes them in the given order (the proxy page puts `mode` last). |
| 52 | Apply: recorded in section 5.8. |
| 53 | Apply: api-methods fragments carry `"secret": true`; the loader keeps the flag; runtime never logs args, error data or dry-run toasts of such methods; the harness redaction is I's. |
| 54 | Apply (B0): `system.power.settings.get` (not dangerous) and `.set` (dangerous) are added to `host/src/api-methods.json`. |
| 55 | Apply (B0): `events.rs` prefixes, `gnome.rs` relays, per-batch i18n allow files. |
| 56 | Apply (B0): `caps.get` reports `session` ("gnome", "hyprland" or "other") and `gnome`; GNOME-only pages declare `needs: "gnome"`. |
| 57 | Apply (C for `system.network.setAirplane`, in `host/src/services/system/network.rs`; B for Bluetooth power, in `host/src/services/system/bluetooth.rs`). |
| 58 | Apply (A): setting `start.windowsKey` (B0 adds the default `true`), read by `extension/shortcutBridge.js` (A); GNOME's `overlay-key` stays a separate row marked "Alleen GNOME-sessie". |
| 59 | Apply (E), Locales: `machine.locales` lists the generated locales (`localedef --list-archive` through `system_tool`), as gnome-desktop does; `language-pack-gnome-nl` is not installed here, so `SetLanguage nl_NL` is only useful after `machine.languages.install`: order the UI that way. |
| 60 | Reject the split into three workflow steps: this workflow builds B0 as one step and the task puts the mirror in B0. Its intent is kept: B0 is committed in logical steps (scaffolding, foundation, mirror), the three rendering pairs go to A (49), and every consumer inside the multi-monitor worktree's files is an integration note (section 8). |
| 61 | Apply: (a) the existing `display.tsv`, `storage.tsv`, `notifications.tsv`, `apps.tsv` stay; their batches append to them. (b) `tests/gnome/lib/standins/__init__.py` (builder, B0) plus `tests/gnome/lib/standins/<batch>.py` per batch. (c) the harness stubs include `nmcli`, `pkexec` (logs argv), `localedef`, `ubuntu-insights`, `snap-store`, `gnome-software`, `update-manager`, `grdctl`, `tecla`, `bluetooth-sendto`, `passwd`, `simple-scan`, `orca`, `ibus`, `gnome-remote-desktop-enable-service`. (d) B0 freezes the home.rs helper signatures with a compile test (`pages/mod.rs` `home_api`). (e) A's kit changes are additive and land first within A. |
| 62 | Apply (D; the VALIDATION note is I's): the harness buses run no PipeWire; start pipewire, wireplumber and pipewire-pulse (all installed) inside the harness for `settings-system.py`, or the sound pages are covered by unit tests only. `VALIDATION_SETTINGS.md` states that the harness checks keys, not gsd effects. |
| 63 | Apply (F), test for 4: with per-app `enable-sound-alerts` true a GNOME notification plays exactly one sound while the WinPoint toast shows, none when `notifications.sounds` is false or DND is on. |
| 64 | Apply (D), test for 32: a persistent apply shows exactly one keep-changes dialog; on timeout the previous configuration comes back. |
| 65 | Apply (B0, done): registry tests: data pages excluded from search, Start and recent lists; every `needs` value a known caps key; nl "muis" and en "mouse" find `mouse` before `a11y-mouse`. |
| 66 | Apply (I): `win11-app --gnome-look` restores accent-color; nothing any batch wrote landed in the ubuntu-dock, ding or tiling-assistant schemas; `org.gnome.shell enabled-extensions` and mutter `experimental-features` are unchanged. |
| 67 | Apply (A), test for 10: the touchpad page has no sensitivity dropdown; `disable-while-typing-timeout` is in no allow table (B0's audit already asserts the second); it stays out unless a later mutter reads it. |
| 68 | Apply (E), test for 30: on a machine without timesync1, Date & time shows no error toast and "Nu synchroniseren" calls only timedate1 `SetNTP`; `settings-time.py` with the stand-in. |
| 69 | Apply (D): the 15 s keep dialog moves from D's Settings code to the bridge path of 32; Hyprland keeps its `keep_changes_dialog`; both paths share the nl/en strings "Deze beeldscherminstellingen behouden?", "Wijzigingen behouden", "Terugzetten". |

## 2. The page tree

Columns: id, parent, module (`native/apps/settings/src/pages/<module>.rs`), owner, `needs` (a caps key, section
6.2), D = data page (needs a parameter), Windows page, nl title / en title. Navigation order is the order below (the
registry's order). Third level is the deepest (top › parent › page).

### Top level (navigation pane)

| id | module | owner | Windows | nl / en |
|----|--------|-------|---------|---------|
| home | home | keep | Home | Startpagina / Home |
| system | system | E (B0 the child list) | System | Systeem / System |
| bluetooth | bluetooth | B | Bluetooth & devices | Bluetooth en apparaten / Bluetooth & devices |
| network | network | C | Network & internet | Netwerk en internet / Network & internet |
| personalization | personalization | F (B0 the child list) | Personalisation | Persoonlijke instellingen / Personalisation |
| apps | apps | F (B0 the child list) | Apps | Apps / Apps |
| accounts | accounts | E (B0 the child list) | Accounts | Accounts / Accounts |
| time | time | E (B0 the child list) | Time & language | Tijd en taal / Time & language |
| accessibility | accessibility | A (B0 the child list) | Accessibility | Toegankelijkheid / Accessibility |
| privacy | privacy | F (B0 the child list) | Privacy & security | Privacy en beveiliging / Privacy & security |
| update | update | keep | Windows Update | Update en herstel / Update & recovery |

### Systeem

| id | parent | module | owner | needs | D | Windows | nl / en |
|----|--------|--------|-------|-------|---|---------|---------|
| display | system | display | D | | | System › Display | Beeldscherm / Display |
| display-advanced | display | display | D | gnome | | › Display › Advanced display | Geavanceerd beeldscherm / Advanced display |
| nightlight | display | nightlight | D | gnome | | › Display › Night light | Nachtlampje / Night light |
| colorprofile | display | colorprofile | B | | | › Display › Colour profile | Kleurprofiel / Colour profile |
| sound | system | sound | D | | | System › Sound | Geluid / Sound |
| sound-device | sound | sound_device | D | | D | › Sound › Properties | Eigenschappen / Properties |
| sound-devices | sound | sound_device | D | | | › Sound › All sound devices | Alle geluidsapparaten / All sound devices |
| volume-mixer | sound | volume_mixer | D | | | › Sound › Volume mixer | Volumemixer / Volume mixer |
| notifications | system | notifications | F | | | System › Notifications | Meldingen / Notifications |
| app-notifications | notifications | app_notifications | F | | D | › Notifications › (app) | Meldingen van app / App notifications |
| screentime | system | screentime | D | gnome | | (Focus sibling) | Schermtijd / Screen time |
| power | system | power | D | | | System › Power & battery | Energie en batterij / Power & battery |
| storage | system | storage | D | | | System › Storage | Opslag / Storage |
| storagesense | storage | storagesense | D | | | › Storage › Storage Sense | Opslaginzicht / Storage Sense |
| multitasking | system | multitasking | D | gnome | | System › Multitasking | Multitasking / Multitasking |
| system-advanced | system | system_advanced | C | | | System › Advanced | Geavanceerd / Advanced |
| remotedesktop | system | remotedesktop | C | gnome | | System › Remote Desktop | Extern bureaublad / Remote Desktop |
| clipboard | system | clipboard | keep | | | System › Clipboard | Klembord / Clipboard |
| about | system | about | E | | | System › About | Info / About |

### Bluetooth en apparaten

| id | parent | module | owner | needs | D | Windows | nl / en |
|----|--------|--------|-------|-------|---|---------|---------|
| bluetooth-devices | bluetooth | bt_devices | B | | | › Devices | Apparaten / Devices |
| printers | bluetooth | printers | B | | | › Printers & scanners | Printers en scanners / Printers & scanners |
| printer | printers | printers | B | | D | › Printers & scanners › (printer) | Printer / Printer |
| mouse | bluetooth | mouse | A | | | › Mouse | Muis / Mouse |
| touchpad | bluetooth | touchpad | A | touchpad | | › Touchpad | Touchpad / Touchpad |
| pen | bluetooth | pen | A | tablet | | › Pen & Windows Ink | Pen en tablet / Pen & tablet |
| autoplay | bluetooth | autoplay | B | | | › AutoPlay | Automatisch afspelen / AutoPlay |
| usb | bluetooth | usb | B | thunderbolt | | › USB | USB / USB |

### Netwerk en internet

| id | parent | module | owner | needs | D | Windows | nl / en |
|----|--------|--------|-------|-------|---|---------|---------|
| wifi | network | network | C | | | › Wi-Fi | Wi-Fi / Wi-Fi |
| wifi-network | wifi | wifi_network | C | | D | › Wi-Fi › <SSID> properties | Wi-Fi-eigenschappen / Wi-Fi properties |
| wifi-known | wifi | wifi_network | C | | | › Wi-Fi › Manage known networks | Bekende netwerken beheren / Manage known networks |
| ethernet | network | network | C | | | › Ethernet | Ethernet / Ethernet |
| mobile | network | mobile | C | modem | | › Cellular | Mobiel netwerk / Mobile network |
| vpn | network | vpn | C | | | › VPN | VPN / VPN |
| hotspot | network | hotspot | C | | | › Mobile hotspot | Mobiele hotspot / Mobile hotspot |
| flightmode | network | flightmode | C | | | › Flight mode | Vliegtuigmodus / Flight mode |
| proxy | network | proxy | C | | | › Proxy | Proxy / Proxy |
| network-advanced | network | network_advanced | C | | | › Advanced network settings | Geavanceerde netwerkinstellingen / Advanced network settings |
| sharing-advanced | network-advanced | network_advanced | C | | | › Advanced network settings › Advanced sharing settings | Geavanceerde instellingen voor delen / Advanced sharing settings |

`net_editor.rs` (C) holds the shared IP/DNS/security/802.1X dialogs; it serves no page.

### Persoonlijke instellingen, Apps, Accounts

| id | parent | module | owner | needs | D | Windows | nl / en |
|----|--------|--------|-------|-------|---|---------|---------|
| background | personalization | background | F | | | › Background | Achtergrond / Background |
| colors | personalization | colors | F | | | › Colours | Kleuren / Colours |
| themes | personalization | themes | keep | | | › Themes | Thema’s / Themes |
| start | personalization | start | keep | | | › Start | Start / Start |
| taskbar | personalization | taskbar | multi-monitor | | | › Taskbar | Taakbalk / Taskbar |
| desktopicons | personalization | desktopicons | keep (B0 line) | | | › Themes › Desktop icon settings | Bureaubladpictogrammen / Desktop icons |
| installed | apps | installed | F | | | › Installed apps | Geïnstalleerde apps / Installed apps |
| app-advanced | installed | app_advanced | F | | D | › Installed apps › Advanced options | Geavanceerde opties / Advanced options |
| defaultapps | apps | defaultapps | F | | | › Default apps | Standaard-apps / Default apps |
| startup | apps | startup | keep | | | › Startup | Opstarten / Startup |
| yourinfo | accounts | yourinfo | E | | | › Your info | Je info / Your info |
| email-accounts | accounts | email_accounts | E | | | › Email & accounts | E-mail en accounts / Email & accounts |
| signin | accounts | signin | E | | | › Sign-in options | Aanmeldingsopties / Sign-in options |
| otherusers | accounts | otherusers | E | | | › Other users | Andere gebruikers / Other users |

### Tijd en taal, Toegankelijkheid, Privacy, Update

| id | parent | module | owner | needs | D | Windows | nl / en |
|----|--------|--------|-------|-------|---|---------|---------|
| datetime | time | time | E | | | › Date & time | Datum en tijd / Date & time |
| language | time | time | E | | | › Language & region | Taal en regio / Language & region |
| language-options | language | keyboards | A | | | › Language & region › Language options | Taalopties / Language options |
| formats | language | formats | E | | | › Regional format › Change formats | Regionale notatie / Regional format |
| language-admin | language | language_admin | E | | | › Administrative language settings | Beheerinstellingen voor taal / Administrative language settings |
| typing | time | typing | A | | | › Typing | Typen / Typing |
| keyboard-advanced | typing | keyboards | A | | | › Typing › Advanced keyboard settings | Geavanceerde toetsenbordinstellingen / Advanced keyboard settings |
| shortcuts | typing | shortcuts | A | gnome | | (WinPoint page) | Sneltoetsen / Keyboard shortcuts |
| textsize | accessibility | accessibility | A | | | › Text size | Tekstgrootte / Text size |
| visualeffects | accessibility | accessibility | A | | | › Visual effects | Visuele effecten / Visual effects |
| mousepointer | accessibility | a11y_vision | A | | | › Mouse pointer and touch | Muisaanwijzer en aanraken / Mouse pointer and touch |
| textcursor | accessibility | a11y_vision | A | | | › Text cursor | Tekstcursor / Text cursor |
| magnifier | accessibility | a11y_vision | A | gnome | | › Magnifier | Vergrootglas / Magnifier |
| contrast | accessibility | accessibility | A | | | › Contrast themes | Contrastthema’s / Contrast themes |
| narrator | accessibility | a11y_vision | A | gnome | | › Narrator | Verteller / Narrator |
| a11y-audio | accessibility | a11y_interaction | A | | | › Audio | Audio / Audio |
| a11y-keyboard | accessibility | a11y_interaction | A | | | › Keyboard | Toetsenbord / Keyboard |
| a11y-mouse | accessibility | a11y_interaction | A | | | › Mouse | Muis / Mouse |
| security | privacy | privacy_security | F | | | › Windows Security | Apparaatbeveiliging / Device security |
| diagnostics | privacy | privacy_security | F | | | › Diagnostics & feedback | Diagnostische gegevens en feedback / Diagnostics & feedback |
| activity | privacy | privacy_history | F | | | › Activity history | Activiteitengeschiedenis / Activity history |
| searchpermissions | privacy | privacy_history | F | | | › Search permissions | Zoekmachtigingen / Search permissions |
| searching | privacy | privacy_history | F | tracker | | › Searching Windows | Deze pc doorzoeken / Searching this PC |
| location | privacy | privacy_perms | F | | | › Location | Locatie / Location |
| camera | privacy | privacy_perms | F | | | › Camera | Camera / Camera |
| microphone | privacy | privacy_perms | F | | | › Microphone | Microfoon / Microphone |
| update-history | update | update_history | keep | | | › Update history | Updategeschiedenis / Update history |
| update-advanced | update | update_advanced | keep | | | › Advanced options | Geavanceerde opties / Advanced options |

91 pages. Titles follow the plan except Opslaginzicht (the plan's "Opslagzin"): tools/i18n/glossary.tsv already holds
Windows 11's verified "Opslaginzicht = Storage Sense". Each new page id has a registry entry (glyph, nl/en description and keywords), a builder (B0: the
module's `PAGES` table points at `common::placeholder` until its batch replaces it) and, unless it is a data page,
a row in `native/shell/src/start/settings_pages.json` (regenerated with `python3 -B tools/settings-pages.py --write`).

Aliases (`registry::ALIASES`, deep links and ms-settings names): `nightlight` and `night-light` now open
`nightlight`; `sound-devices`, `yourinfo`, `storagesense`, `bluetoothdevices` name pages of their own; new:
mousetouchpad → mouse, devices-touchpad → touchpad, pen, autoplay, usb, network-vpn → vpn, network-proxy → proxy,
network-mobilehotspot → hotspot, network-airplanemode → flightmode, network-cellular → mobile, network-wifi → wifi,
network-ethernet → ethernet, network-wifisettings → wifi-known, network-advancedsettings → network-advanced,
colorprofile, apps-volume → volume-mixer, multitasking, remotedesktop, developers and optionalfeatures →
system-advanced, emailandaccounts → email-accounts, signinoptions → signin, otherusers, regionformatting →
formats, typing, keyboard → keyboard-advanced, easeofaccess-display → textsize, easeofaccess-visualeffects,
-mousepointer, -cursor, -magnifier, -highcontrast, -narrator, -audio, -keyboard, -mouse, privacy-location →
location, privacy-webcam → camera, privacy-microphone → microphone, privacy-feedback → diagnostics,
privacy-activityhistory → activity, search-permissions → searchpermissions, cortana-windowssearch → searching,
windowsdefender → security, appsfeatures-app → installed (never a data page), storagepolicies → storagesense,
batterysaver → power. No alias points at a data page (test).

Breadcrumbs: N levels from the parent chain (`registry::ancestors`); the navigation pane highlights the top-level id
(`registry::top_of` walks the chain).

## 3. Rules for the shared files (after B0)

After B0's commit these files are frozen for A-F. A batch that needs a change in one writes a note in its handoff
(`notes/claude/settings-<batch>.md`, section "Notes for I"); I applies the notes. Nothing else in a batch's diff may
touch them.

* `native/apps/settings/src/registry.rs`: every entry, `NEEDS`, `DATA_PAGES` and `ALIASES` are final. Registry
  notes (a keyword, a description) go to I.
* `native/apps/settings/src/pages/mod.rs`: every `pub mod`, `MODULES` and `MODULE_CSS` entry exists; a batch fills
  its modules' `PAGES` tables and `CSS` constants in its own files.
* `native/apps/settings/src/{app.rs, main.rs, common.rs, gs.rs, pickers.rs, widgets.rs, preview.rs, navicons.rs,
  colorpicker.rs, keepalive.rs}`: frozen. A helper a batch needs lives in its own module (`pub(super)`); a bug in a
  B0 helper is a note for I (with the fix as a patch).
* `host/src/services/mod.rs`: every prefix, `LazyService` slot and `shutdown` exists. A batch fills its service
  files.
* `host/src/runtime.rs`, `host/src/lib.rs`, `host/src/events.rs`, `host/src/gnome.rs`, `host/src/tools.rs`,
  `host/src/dlib.rs`, `host/src/settings_mirror.rs`, `host/src/settings_effects.rs`,
  `host/src/services/system/common.rs`, `host/src/services/system.rs`, `host/src/services/caps.rs`,
  `host/src/services/gsettings.rs` and `host/src/services/gsettings/{mod-level files, value.rs, owned.rs, tests.rs,
  allow/mod.rs}`: frozen.
* `host/src/i18n.rs`: STEMS is final. Only E may edit the language resolution (amendment 18).
* `host/src/api-methods.json`: frozen (B0 fixed the power drift). Each batch lists its methods in its own
  `host/src/api-methods/<fragment>.json` only: `[{"name", "dangerous", "doc", "secret"?}]`, names unique across all
  files (the loader refuses duplicates at start, and a test checks it), the union is served sorted by name.
* `host/src/settings-defaults.json`: frozen. B0 added every key the plan needs (section 6.5). A new key is a note.
* `host/src/settings.rs`: only E (amendment 18). The multi-monitor branch changes it too (section 4.0: `CHOICES`,
  `choice_ok` in `type_ok`, a test at the end of `mod tests`): E adds its tests in a module of its own (`mod
  language_tests` after `mod tests`), never at the end of `mod tests`, and keeps `type_ok` as it is.
* `native/shell/src/start/settings_pages.json`: regenerated only by I (`tools/settings-pages.py --write`).
* `native/kit/src/glyph_data.rs`: frozen (B0 added touchpad, pen, vpn, magnifier, narrator, text-cursor,
  remote-desktop). Other kit files: only A, and only `theme.rs`, `ctx.rs`, `controls/toggles.rs` (additive); and E's
  `i18n.rs` (amendment 18). Every kit reader of settings.language calls `kit::i18n::resolve(&value)` (B0: ctx.rs at
  start and in `apply_look`, `set_forced_lang`), so E changes only `resolve` for `"system"` and never touches ctx.rs.
* `extension/launchBridge.js`: frozen (generic, B0). `extension/displayBridge.js` is D's, `extension/inputBridge.js`
  A's, `extension/notificationsBridge.js` F's, `extension/shortcutBridge.js` A's.
* Test infrastructure: `tests/gnome/harness.py`, `tests/gnome/lib/scenario.py`, `tests/gnome/lib/standins/__init__.py`,
  `tests/dbus-fakes/run.sh`, `tests/rust-sandbox/run-tests.sh`, `tools/i18n/*.py`, `tools/settings-pages.py`:
  frozen. Per batch: `tests/gnome/lib/standins/<batch>.py`, `tests/dbus-fakes/fakes/<batch>/`,
  `tests/gnome/scenarios/settings-<x>.py`, `tests/i18n/BASELINE-CHANGES.rust-settings-<batch>`,
  `tests/i18n/allow-host-settings-<batch>.txt`, `tests/i18n/allow-native-settings-<batch>.txt`,
  `tests/i18n/windows-glossary/<batch>.tsv` (B0 created each with a header).
* `tools/i18n/glossary.tsv`, `tools/i18n/glossary-context.tsv`, `tests/i18n/dutch-words.txt`: frozen; in native
  code a pair may carry `// glossary-ok: Term`; anything else is a note for I.
* `VALIDATION_SETTINGS.md`, `CHANGELOG.md`, `README.md`, `README.nl.md`: only I.

Stems that already existed keep their owner: `display.tsv` and `storage.tsv` (D), `notifications.tsv` and `apps.tsv`
(F), `system.tsv` (frozen; a batch that edits a `host/src/services/system/*.rs` file it owns adds its rows to its own
new stem, e.g. D's audio rows to `sound.tsv`).

## 4. File ownership

### 4.0 The multi-monitor worktree (not a batch here; never touched by B0 or A-F)

`/run/media/robin/22TB/win11-gnome-wt/mm`, branch `claude/multi-monitor` (89ab5d99 on 16-11, based on 1c14ff4a).
It owns, and no batch edits: `native/apps/settings/src/pages/taskbar.rs`, `native/shell/src/panel/**`,
`native/shell/src/desktop/**`, `native/shell/src/flyouts/menus.rs`, `native/shell/src/flyouts/menus/menu.rs`,
`native/shell/src/wayland.rs`, `native/shell/css/menus.css`, `native/shell/css/taskbar.css`, `native/ARCH.md`,
`extension/panelBridge.js`, `extension/desktopBridge.js`, `extension/frostedBridge.js` and
`tests/gnome/scenarios/multi-monitor.py` with the `*.mm`/`multi-monitor*.out` results. Changes there that this plan
needs are handoff patches applied by I after the multi-monitor merge (section 8).

Files it shares with this plan (its diff of 89ab5d99 against 1c14ff4a; `git merge-tree --write-tree` of B0's last
commit and 89ab5d99 is clean):

* `host/src/settings-defaults.json` (B0, frozen): it adds `taskbar.allDisplays` and `taskbar.multiDisplayApps`
  after `taskbar.autoHide`; B0's keys are elsewhere. Merges cleanly.
* `host/src/settings.rs` (E): `CHOICES` and `choice_ok` in `type_ok`, and a test at the end of `mod tests`; E's
  rule is in section 3.
* `tests/gnome/harness.py`, `tests/gnome/lib/scenario.py` (B0, frozen): `--second-monitor` and the monitor helpers.
  Merge cleanly.
* `tests/gnome/README.md`: both add helper rows. B0's settings-parity helpers are a table of their own after the
  "Settings parity" paragraph (moved there by the B0 review, finding 11), so the multi-monitor rows in the main
  helper table no longer conflict.
* `README.md`, `README.nl.md`, `CHANGELOG.md` (I only).

I checks this list again with `git merge-tree --write-tree <I's base> claude/multi-monitor` before merging; a file the
multi-monitor branch added since 89ab5d99 is its own, as above.

### 4.1 B0 (Foundation)

Everything section 3 lists as frozen, plus the stubs it creates for the batches. Native:
`registry.rs`, `app.rs`, `main.rs`, `common.rs` (child_links, placeholder, caps cache), `gs.rs`, `pickers.rs`,
`goa_helper.rs` (stub), `pages/mod.rs`, the child-list loops of `pages/{system,time,personalization,apps,accounts,
privacy,accessibility}.rs`, the line in `pages/desktopicons.rs`, every new module as a stub, `native/kit/src/
glyph_data.rs`, `native/shell/src/start/settings_pages.json`, `native/shell/src/flyouts/calendar.rs` and
`native/calendar/src/{config.rs, ui/mod.rs}` (week numbers, first day -1). Host: `services/mod.rs`,
`services/gsettings.rs` + `services/gsettings/{value.rs, owned.rs, tests.rs, allow/*.rs}`, `services/caps.rs`, the
13 stub service files, `services/system/common.rs` (`call_interactive`), `tools.rs`, `dlib.rs`, `settings_mirror.rs`
+ `settings_mirror/{tests.rs, render.rs (stub)}`, `lib.rs`, `settings_effects.rs` (hook), `settings_runtime.rs`
(mirror start), `settings-defaults.json`, `runtime.rs`, `api-methods.json` (power drift), `api-methods/*.json`,
`events.rs`, `gnome.rs`, `i18n.rs`, `host/i18n/{core.tsv rows, 15 new stems}`. Extension: `launchBridge.js`,
`displayBridge.js` and `inputBridge.js` (stubs). Tests and tools as section 3.

### 4.2 A: Input & accessibility

Pages mouse, touchpad, pen, typing, language-options, keyboard-advanced, shortcuts, accessibility, textsize,
visualeffects, contrast, mousepointer, textcursor, magnifier, narrator, a11y-audio, a11y-keyboard, a11y-mouse.
Files: `pages/{mouse,touchpad,pen,keyboards,typing,shortcuts,accessibility,a11y_vision,a11y_interaction}.rs`;
`host/src/services/input.rs` + `host/src/services/input/*`; `host/src/services/gsettings/allow/input.rs`;
`host/src/settings_mirror/render.rs`; `host/src/api-methods/input.json`; `host/i18n/input.tsv`;
`extension/inputBridge.js`, `extension/shortcutBridge.js`; `native/kit/src/{theme.rs, ctx.rs,
controls/toggles.rs}`; `native/shell/src/theme.rs`, `native/shell/src/flyouts/quick.rs`;
`native/calendar/src/theme.rs` (the Calendar app is no kit app: its theme.rs multiplies its CSS by
`accessibility.textScale` itself, through config.rs `text_scale`; once A's textScale pair puts the size in GNOME's
text-scaling-factor, GTK scales win11-calendar already, so A makes calendar/theme.rs stop multiplying, as it does
in kit/theme.rs and shell/theme.rs; A leaves config.rs alone, E has its language line);
`tests/gnome/scenarios/settings-input.py`, `settings-a11y.py`. Handoff patches for I:
`native/shell/src/desktop/grid.rs` (text scale), `native/shell/src/panel/taskbar/input.rs` (input indicator).

### 4.3 B: Bluetooth, printers & colour

Pages bluetooth, bluetooth-devices, printers, printer, autoplay, usb, colorprofile. Files:
`pages/{bluetooth,bt_devices,printers,autoplay,usb,colorprofile}.rs` (keep the home.rs helper signatures);
`host/src/services/devices.rs` + `devices/*`; `host/src/services/printers.rs` + `printers/*`;
`host/src/services/system/bluetooth.rs` (amendment 57); `allow/devices.rs`; `api-methods/devices.json`;
`host/i18n/{devices,printers}.tsv`; `tests/gnome/lib/fake_ipp.py`, `tests/gnome/scenarios/settings-devices.py`.

### 4.4 C: Network & sharing

Pages network, wifi, ethernet, wifi-network, wifi-known, mobile, vpn, hotspot, flightmode, proxy,
network-advanced, sharing-advanced, remotedesktop, system-advanced. Files: `pages/{network,wifi_network,net_editor,
vpn,hotspot,flightmode,proxy,mobile,network_advanced,remotedesktop,system_advanced}.rs`;
`host/src/services/net.rs` + `net/*`; `host/src/services/sharing.rs` + `sharing/*`;
`host/src/services/system/network.rs` (amendment 57); `allow/network.rs`; `api-methods/network.json`;
`host/i18n/{net,sharing}.tsv`; `tests/gnome/scenarios/settings-network.py`, `settings-sharing.py`.

### 4.5 D: System

Pages display, display-advanced, nightlight, sound, sound-device, sound-devices, volume-mixer, power, storage,
storagesense, multitasking, screentime. Files: `pages/{display,nightlight,sound,sound_device,volume_mixer,power,
storage,storagesense,multitasking,screentime}.rs`; `host/src/services/display.rs` + `display/*`;
`host/src/services/sound.rs` + `sound/*`; `host/src/services/{battery,wellbeing}.rs`;
`host/src/services/storage.rs` + `storage/*`; `host/src/services/system/{audio.rs, audio_monitor.rs, power.rs,
energy.rs, hardware.rs}`; `allow/system.rs`; `api-methods/system.json`; `host/i18n/{sound,battery,wellbeing,
display,storage}.tsv`; `extension/displayBridge.js`; `native/shell/src/flyouts/menus/taskview.rs`
(`taskView.allDesktops`); `tests/gnome/scenarios/settings-system.py`.

### 4.6 E: Accounts, time & region, About

Pages system (rename dialog, `device_hero` signature kept), accounts, yourinfo, email-accounts, signin,
otherusers, time, datetime, language, formats, language-admin, about. Files: `pages/{system,accounts,yourinfo,
email_accounts,signin,otherusers,time,formats,language_admin,about}.rs`; `native/apps/settings/src/goa_helper.rs`;
`host/src/services/users.rs` + `users/*`; `host/src/services/goa.rs`; `host/src/services/machine.rs` +
`machine/*`; `host/src/services/system/info.rs`; `host/src/settings.rs` and the language resolution of
`host/src/i18n.rs` and `native/kit/src/i18n.rs` (amendment 18: `kit::i18n::resolve`, section 3), plus the two
native readers outside the kit: `native/shell/src/core.rs` `Core::lang()` and the `en` line of
`native/calendar/src/config.rs` `load()` (neither crate links the kit; nobody else edits those lines);
`allow/accounts.rs`; `api-methods/accounts.json`;
`host/i18n/{users,goa,machine}.tsv`; `tests/gnome/scenarios/settings-accounts.py`, `settings-time.py`.

### 4.7 F: Apps, notifications, privacy & background

Pages apps, installed, app-advanced, defaultapps, notifications, app-notifications, privacy, security, diagnostics,
activity, searchpermissions, searching, location, camera, microphone, personalization, background, colors.
Files: `pages/{apps,installed,app_advanced,defaultapps,notifications,app_notifications,privacy,privacy_security,
privacy_history,privacy_perms,personalization,background,colors}.rs`; `host/src/services/privacy.rs` +
`privacy/*`; `host/src/services/background.rs`; `host/src/services/notifications.rs`; `host/src/apps/defaults.rs`;
`allow/apps.rs`; `api-methods/apps.json`; `host/i18n/{privacy,background,notifications,apps}.tsv`;
`extension/notificationsBridge.js`; `tests/gnome/scenarios/settings-apps.py`, `settings-privacy.py`.

### 4.8 I: Integration (alone, last)

Merges A-F (any order; a textual conflict is a plan violation that goes back to its batch), applies the notes,
regenerates `settings_pages.json`, removes `common::placeholder` (a test asserts no page uses it), applies the
multi-monitor patches (section 8), `VALIDATION_SETTINGS.md`, `CHANGELOG.md`, READMEs, `settings-tour.py`.

## 5. Host API per batch

Notation: `name {args} → result`; `!` = dangerous (listed with `"dangerous": true`; skips its write under
`ctx.dry_run`; emits the batch's changed event); `🔑` = secret (`"secret": true`: args, error data and dry-run
toasts are never logged). Each prefix is a `LazyService` in `host/src/services/mod.rs`; B0's stub returns `None`
for every method (so the runtime answers `unknown_method`) until its batch fills it. Every service lists its
methods in a `METHODS: &[(&str, bool)]` table (name, dangerous) that B0 tests compare with the api-methods files in
both directions (`runtime.rs` `methods_tests`: each table name listed with its flag, each fragment name in its
service's table). The existing services the batches extend have theirs too, already holding their old methods:
`services/display.rs` and `services/storage.rs` (D adds `display.*`, `storage.cleanTemp`),
`services/notifications.rs` (F adds `notifications.apps`, `notifications.app.set`), and `host/src/apps/mod.rs`
(`apps.*`, which its dispatch reads; frozen, F adds no apps method). Errors use `ApiError::tr` with the batch's own stems. Argument names below are the contract; a batch may
add optional arguments (documented in its fragment's `doc`).

### 5.1 B0 foundation (`api-methods/foundation.json`, stems `gsettings`, `caps`, core rows)

* `gsettings.get {keys: [{schema, key, path?}]}` → `{items: [Item]}`.
* `gsettings.set! {schema, path?, values: {key: value}}` or `{writes: [{schema, path?, values}]}` → `{items}`.
* `gsettings.reset! {schema, path?, keys: [key]}` → `{items}`.
* `caps.get {}` → the caps object (6.2).
* Events: `gsettings.changed {schema, path, key, item}`, `caps.changed {caps}`.

`Item` = `{schema, path, key, allowed, available, writable, mode, setting, method, value, default, isDefault, type,
range}`; `mode` is `"write" | "read" | "service" | "owned" | "mirrored"`; `setting` is the WinPoint settings path of
an owned or mirrored key, `method` the host method of a service key; `type` is the GVariant type string or
`"enum"`/`"flags"`; `range` is `{choices: [...]}` for enum and flags, `{min, max}` for a range, else null.

### 5.2 A: `input.*` (`api-methods/input.json`, stem `input`, allowlist `allow/input.rs`)

* `input.layouts {}` → `{layouts: [{id, name, short, languages, countries}], languages: [{code, name}]}`
* `input.engines {}` → `{engines: [{id, name, language, setup}]}`
* `input.sources.set! {sources: [[type, id]]}` (also the async pkexec `set-debian-keyboard <layouts> <variants>
  [options]`, characters `[a-zA-Z0-9,_:-]`)
* `input.options.set! {lv3?: string|null, compose?: string|null}`
* `input.preview {layout}` (tecla through `tools::system_tool`)
* `input.engineSetup {id}` (through `launch.ibusSetup {engine: id}`, section 6.6)
* `input.capture {timeout?}` → `{accel}`, `{disabled: true}` (a plain Backspace: the shortcut is turned off) or
  `{cancelled: true}` (Esc, the timeout, or `input.capture.cancel`) (inputBridge `Main.pushModal`, one key event,
  release; amendment 44). The host waits longer than any other bridge call: A calls the bridge with
  `gnome::call_api_with_timeout(method, args, timeout_ms)` (B0), `timeout_ms` above the capture's own timeout
  (default 30 s), never `gnome::call` (3 s). `pickers::KeyCapture` treats an error as "no host capture" and listens
  itself.
* `input.capture.cancel {}` → `{}`: inputBridge releases the modal grab and answers the pending capture with
  `{cancelled: true}`. KeyCapture sends it when its dialog closes before the host answered.
* `input.shortcuts {}` → `{sections: [{id, title, items: [{schema, key, path?, name, accels, default, held,
  editable}]}], custom: [{id, name, command, binding}]}`
* `input.shortcuts.set! {schema, key, path?, accels, replace?}` → `{conflict?: {schema, key, name}}`
* `input.shortcuts.reset! {schema, key, path?}`, `input.shortcuts.resetAll! {}`
* `input.shortcuts.custom.add! {name, command, binding}` → `{id}`; `.update! {id, name?, command?, binding?}`;
  `.remove! {id}`
* `input.tablets {}` → `{tablets: [{vidpid, name, kind, buttons, rings, strips, styli: [serial]}]}`
* `input.tablet.mapButtons {vidpid}` (pad OSD through inputBridge)
* Events: `input.changed {what}`; bridge events `input.*` (relayed by `gnome.rs`).

### 5.3 B: `devices.*`, `printers.*` (`api-methods/devices.json`, stems `devices`, `printers`, `allow/devices.rs`)

* `devices.bluetooth.adapter {}` → `{address, alias, powered, discoverable}`; `devices.bluetooth.setDiscoverable!
  {on}`; `devices.bluetooth.sendFiles {address, name}`
* `devices.batteries {}` → `{devices: [{id, kind, model, percentage, state, icon}]}`
* `devices.thunderbolt.get {}`; `.setAuthMode! {mode}`; `.enroll! {uid, policy}`; `.authorize! {uid}`;
  `.forget! {uid}`
* `devices.color.get {}`; `.setEnabled! {device, on}`; `.makeDefault! {device, profile}`; `.addProfile! {device,
  profile}`; `.importProfile! {device, path}`; `.removeProfile! {device, profile}`; `.installSystemWide! {profile}`;
  `.calibrate.start! {device, quality, kind, whitepoint, title}`; `.calibrate.cancel! {}`; `.calibrate.resume! {}`
* `printers.list {}`; `printers.get {name}`; `printers.discover {address?, username?, password?}` 🔑;
  `printers.drivers {deviceId?, makeModel?}`; `printers.jobs {name?, which?}`; `printers.testPage! {name}`;
  `printers.cleanHeads! {name}`; `printers.add! {name, uri, ppd?, ppdFile?, info?, location?}`;
  `printers.rename! {name, newName}`; `printers.setLocation! {name, location}`; `printers.setDefault! {name}`;
  `printers.setEnabled! {name, on}`; `printers.setAcceptJobs! {name, on}`; `printers.remove! {name}`;
  `printers.setOption! {name, option, values}`; `printers.job.cancel! {id}`; `printers.job.hold! {id}`;
  `printers.job.release! {id}`; `printers.job.priority! {id, priority}`; `printers.jobs.clear! {name}`
* Events: `devices.changed {what}`, `devices.calibrate {state, progress}`, `printers.changed {}`.

### 5.4 C: `net.*`, `sharing.*` (`api-methods/network.json`, stems `net`, `sharing`, `allow/network.rs`)

* `net.connections {type?}`; `net.connection.get {uuid}` (settings as JSON: ssid as bytes, address-data,
  route-data; unknown keys kept); `net.connection.update! {uuid, settings}` (Update2 merge, interactive) 🔑;
  `net.connection.add! {settings}` → `{uuid}` 🔑; `net.connection.delete! {uuid}`; `net.connection.activate!
  {uuid, device?}`; `net.connection.deactivate! {uuid}`
* `net.devices {}`; `net.device.setEnabled! {iface, on}`
* `net.wifi.hidden! {ssid, security, secrets?}` 🔑; `net.wifi.secrets {uuid}` 🔑
* `net.hotspot.get {}` 🔑; `net.hotspot.start! {ssid, password, band, iface?}` 🔑; `net.hotspot.stop! {}`
* `net.radios.set! {wifi?, wwan?}`; `net.connectivity.set! {on}`
* `net.vpn.plugins {}`; `net.vpn.import! {path, type?}` 🔑 (offline libnm importer, then typed AddConnection2 with interactive authorization)
* `net.modem.get {}`; `net.modem.unlock! {modem, pin?, puk?, newPin?}` 🔑; `net.modem.setPin! {modem, enabled,
  pin}` 🔑; `net.modem.changePin! {modem, old, new}` 🔑; `net.modem.setModes! {modem, allowed, preferred}`;
  `net.modem.scan {modem}`; `net.modem.register! {modem, operator?}`; `net.modem.setPrimarySlot! {modem, slot}`;
  `net.modem.apns {mcc, mnc}`
* `sharing.rdp.get {}`; `sharing.rdp.set! {on}`; `sharing.rdp.credentials.get {}` 🔑;
  `sharing.rdp.credentials.set! {username, password}` 🔑; `sharing.rdp.fingerprint {}`;
  `sharing.rdp.system.get {}`; `sharing.rdp.system.set! {on}`; `sharing.rdp.system.credentials.set! {username,
  password}` 🔑
* `sharing.ssh.get {}`; `sharing.ssh.set! {on}`; `sharing.ssh.install! {}`
* `sharing.media.get {}`; `sharing.media.set! {on}`; `sharing.media.folders.set! {folders}`;
  `sharing.media.networks.remove! {uuid}`
* Events: `net.changed {what}`, `sharing.changed {what}`. Existing `system.network.setAirplane` (C) goes through gsd
  Rfkill when present (amendment 57).

### 5.5 D: `display.*`, `sound.*`, `battery.*`, `wellbeing.*`, `storage.cleanTemp` (`api-methods/system.json`)

Stems `sound`, `battery`, `wellbeing`, `display` (existing), `storage` (existing); allowlist `allow/system.rs`.

* `display.state {}`; `display.apply! {serial, method, logical, props?}`; `display.identify {}`;
  `display.confirm! {keep}`; `display.nightLight.status {}`; `display.nightLight.disableUntilTomorrow! {on}`
* `sound.state {}`; `sound.setDefault! {kind, name, port?}`; `sound.setVolume! {kind, index?, name?, volume}`;
  `sound.setMute! {kind, index?, name?, mute}`; `sound.setBalance! {sink, balance?, fade?, lfe?}`;
  `sound.setProfile! {card, profile}`; `sound.setPort! {kind, name, port}`; `sound.moveStream! {kind, index,
  device}`; `sound.setEventVolume! {volume}`; `sound.test {sink?, channel?, alert?}`; `sound.meter {kind, name, on}`;
  `sound.alerts.get {}`; `sound.alerts.set! {alert?, theme?, eventSounds?, startupSound?}`
* `battery.charging.get {}`; `battery.charging.set! {enabled}`; `battery.history {type?, timespan?, resolution?}`
* `wellbeing.history {}`
* `storage.cleanTemp! {}`
* Events: `display.changed {}`, bridge event `display.confirmChange {timeout}`, `display.nightLight.changed`, `sound.changed` (the sound state), `sound.level
  {kind, name, peak}`, `battery.changed` (the charging state). Existing `system.brightness.*` move to DisplayConfig (amendment 31).

### 5.6 E: `users.*`, `goa.*`, `machine.*` (`api-methods/accounts.json`, stems `users`, `goa`, `machine`)

Allowlist `allow/accounts.rs`.

* `users.self {}`; `users.list {}`; `users.setRealName! {uid?, name}`; `users.setIcon! {uid?, path}`;
  `users.faces {}`; `users.setLanguage! {uid?, language}`; `users.setFormats! {uid?, locale}`;
  `users.setAccountType! {uid, type}`; `users.setAutoLogin! {uid, on}`; `users.create! {realName, userName,
  accountType, password?}` 🔑; `users.delete! {uid, removeFiles}`; `users.setPassword! {uid, password?, mode}` 🔑;
  `users.changePassword! {current, new}` 🔑; `users.passwordQuality {password, old?, userName?}` 🔑;
  `users.fingerprint.get {}`; `users.fingerprint.enroll! {finger}`; `users.fingerprint.cancel! {}`;
  `users.fingerprint.delete! {}`
* `goa.list {}`; `goa.providers {}`; `goa.add! {provider}` (spawns `win11-gtk-settings --goa-add <provider>`);
  `goa.setService! {id, service, on}`; `goa.signIn! {id}`; `goa.remove! {id}`
* `machine.info {}`; `machine.setName! {name}`; `machine.time.get {}`; `machine.time.setNtp! {on}`;
  `machine.time.setTime! {usec}`; `machine.time.setTimezone! {zone}`; `machine.time.zones {}`;
  `machine.time.sync! {}`; `machine.locales {}`; `machine.languages {}`; `machine.languages.install! {code}`;
  `machine.login.get {}`; `machine.login.setLocale! {locale}`
* Events: `users.changed {}`, `users.fingerprint.progress {stage, done, result}`, `goa.changed {}`,
  `machine.changed {what}`.

### 5.7 F: `privacy.*`, `background.*`, `notifications.*` additions (`api-methods/apps.json`)

Stems `privacy`, `background`, `notifications` (existing), `apps` (existing); allowlist `allow/apps.rs`.

* `privacy.permissions {table, id}`; `privacy.setPermission! {table, id, app, value}`;
  `privacy.deletePermission! {table, id, app?}`; `privacy.apps.sandboxed {}`
* `privacy.reports.get {}`; `privacy.reports.set! {mode}`
* `privacy.insights.get {}`; `privacy.insights.set! {source, consent}`; `privacy.insights.report {source?}` (or `{cancel: true}`)
* `privacy.security {}`; `privacy.securityCenter.open {}` (through `launch.app`)
* `privacy.searchProviders {}`
* `privacy.app {id}`; `privacy.app.clearCache! {id}`; `privacy.app.resetHandlers! {id}`;
  `privacy.app.removeHandler! {id, type}`; `privacy.app.snapConnect! {id, plug, slot?, connect}`;
  `privacy.app.globalShortcuts.set! {id, shortcuts?, clear?}`
* `background.catalog {}`; `background.add! {path}`; `background.remove! {path}`
* `notifications.apps {}`; `notifications.app.set! {appId, values}` (amendment 6)
* `apps.setDefault` gains the kinds `calendar`, `calls`, `sms` (existing method, `host/src/apps/defaults.rs`).
* Events: `privacy.changed {what}`, `background.changed {}`, `notifications.changed` (existing).

### 5.8 Polkit, as GNOME (amendment 52)

No new root helper and no new .policy. System writes go to daemons that check polkit themselves (hostname1,
timedate1, locale1, AccountsService, fprintd, NetworkManager, ModemManager, BlueZ, UPower, power-profiles-daemon,
colord, bolt, CupsPkHelper, PackageKit, snapd, WhoopsiePreferences, systemd1, org.gnome.RemoteDesktop.Configuration)
through `common::call_interactive`, or through GNOME's pkexec helpers
(`/usr/libexec/gnome-control-center-set-debian-keyboard`, `/usr/libexec/gnome-remote-desktop-enable-service`; both
policies carry `exec.path`). Ubuntu's rules give YES to local active sudo users for `timedate1.set-*`, hostname1
`set-hostname`/`set-static-hostname`, locale1 `set-locale`/`set-keyboard`, `controlcenter.set-keyboard`,
ModemManager `Device.Control`/`SetPrimarySimSlot`, NM `settings.modify.system` (sudo or netdev),
`cupspkhelper.*` (sudo or lpadmin), `whoopsiepreferences.change`. Everything else prompts
(`accounts.user-administration`, `set-login-option`, `systemd1.manage-units`, `bolt.manage`, colord
`install-system-wide`, snapd, PackageKit install, the grd system daemon, fwupd `fix-host-security-attr`). A button
that prompts carries the shield (`pickers::admin_button`).

## 6. The foundation API (B0)

### 6.1 GSettings (`host/src/services/gsettings.rs`)

One worker thread per service with its own GLib MainContext owns every `gio::Settings`; HTTP threads hand it jobs
and wait (10 s). Writes follow `energy.rs`: every key of a call is checked before anything is queued (allowlisted,
mode Write or Mirrored, type, `range_check`, writable, not held), then `delay`, `set_value`, `apply`,
`Settings::sync` and a read-back of every key (a mismatch is `failed gsettings.notTaken`). `ctx.dry_run` validates
only. Values: `b i u q d s as ai ad a(ss) (dd)`, enum (a string), flags (an array of strings); other types are
refused (`bad_args gsettings.badValue`).

Allow tables: `allow/{input,devices,network,system,accounts,apps}.rs`, each a `pub(super) const TABLE: &[Allow]`
with `Allow { schema, path, keys, mode, optional }`. `path` is `None` (the schema's own path) or a relocatable
pattern with typed holes `{appid}` `[a-z0-9-]{1,128}`, `{custom}` `custom[0-9]{1,4}`, `{vidpid}`
`[0-9a-f]{4}:[0-9a-f]{4}`, `{serial}` (amendment 9), `{padctl}` (amendment 9), `{button}` `button[A-Z]`,
`{service}` `[a-z-]{1,32}`. `keys` lists key names (`&["*"]` = every key of the schema). `mode` is `Write`, `Read`,
`Service(method)`, `Owned(setting)` or `Mirrored(setting)`. `optional` names the package whose schema may be
missing (`ubuntu`, `tracker`, `remote-desktop`), so the audit skips it then. A batch edits only its own table.

Refusals (`host/i18n/gsettings.tsv`): `denied gsettings.notAllowed`, `denied gsettings.readOnly`, `denied
gsettings.useMethod` (data.method), `denied gsettings.ownedByWinPoint` (data.setting: the WinPoint look keys written
through `gnome_look::note`: color-scheme, gtk-theme, icon-theme, accent-color, button-layout, every
org.gnome.desktop.background key), `denied gsettings.heldByWinPoint` (data.accel, data.setting; amendment 13),
`denied gsettings.locked`, `bad_args gsettings.badValue` (data.key), `bad_args gsettings.badArgs`, `unsupported
gsettings.unavailable`, `failed gsettings.saveFailed`, `failed gsettings.notTaken`. Never in any table:
ubuntu-dock, DING (`org.gnome.shell.extensions.ding`), tiling-assistant, `org.gnome.shell enabled-extensions` /
`disabled-extensions`, mutter `experimental-features` in a writing mode.

Paths: only GSettings' valid paths (`gsettings::path_ok`: `/` at both ends, no `//`, the characters `[A-Za-z0-9/._:-]`,
at most 512 bytes); anything else is `bad_args gsettings.badArgs` before a settings object exists
(`g_settings_new_full` gives NULL for such a path, and glib-rs does not check that in a release build). A relocatable
schema is opened (and watched) only at a path an allow row matches; `gsettings.get` at any other path reports the
item `allowed: false` without opening it.

Rust API for other services (so a batch's own method shares the checks):
`gsettings::read_item(schema, path, key, lang) -> Result<Item>` and
`gsettings::write_values(method, schema, path, &[(key, Value)], lang, dry_run) -> Result<{items}>`, both on the
caller's thread with the default backend. `write_values` applies `gsettings.set`'s checks with the caller named:
every key needs its allow row at the resolved path (a relocatable row's typed holes, so a path composed from a
method's arguments cannot reach another key) in mode Write, Mirrored or `Service(method)` of that very method;
Read rows are `denied gsettings.readOnly`, Owned rows (WinPoint's look) `denied gsettings.ownedByWinPoint`, a key in
no table `denied gsettings.notAllowed`, a row of another method `denied gsettings.useMethod`; the forbidden
schemas and keys and the held accelerators apply as always.

Every key of a Write or Mirrored row (other than `*`) is named by a control in the committed sources (plan.json,
gnome-a/b.json, windows.json; B0 test `every_written_key_has_a_control_in_the_contracts_sources`). A key a source
names only to say there is no control for it (input-sources `show-all-sources`: "has no UI"; wm.preferences
`audible-bell`: "remains") is in no writing mode. B0's review took out `show-all-sources`, a11y
`always-show-text-caret`, interface `toolkit-accessibility` (gsd's a11y-settings plugin's), `audible-bell` and
media-handling `automount`: a batch that adds a control for one of them adds the row and names the control in its
handoff note.

### 6.2 caps (`host/src/services/caps.rs`)

`caps.get` → `{session, gnome, mouse, touchpad, pointingstick, tablet, touchscreen, thunderbolt, modem,
ambientLight, accelerometer, privacyScreen, backlight, battery, hibernate, fingerprint, colorSensor, geoclue,
tracker, usbguard, sshServer, userShare, malcontent, realmd, rygel, tecla, orca, simpleScan, securityCenter,
insights, whoopsie}`, every value but `session` a bool. Input kinds come from `/run/udev/data` (`E:ID_INPUT_*=1`),
followed by FileMonitors on `/dev/input` and `/run/udev/data` that read them again 300 ms and 1.5 s after the last
event (devtmpfs makes or removes the node before udev writes or deletes its entry; event `caps.changed`); the rest
from /sys, the system bus (500 ms timeouts, cached) and installed files. The registry's `needs` names one of these
keys (the test compares `registry::KNOWN_NEEDS` with `caps::KEYS`); navigation, `child_links` and search hide a
page whose key is false. Settings loads caps once at start and follows `caps.changed`; when the caps that arrive
hide another set of pages (also the first answer, which comes after the first page on a cold start), the page shown
is rebuilt in place if it shows caps-filtered links (an overview page, Home, the search; `common::shows_caps_links`).

### 6.3 Settings-side client (`native/apps/settings/src/gs.rs`, `pickers.rs`, `common.rs`)

* `Gs::load(&p, &[GKey]).await` (one `gsettings.get` per page), `gs.value`, `gs.item`, `gs.available`; controls
  `gs.toggle(k, invert)`, `gs.dropdown(k, options)`, `gs.slider(k, min, max, step, to_ui, from_ui)`,
  `gs.radio(k, options)`, `gs.number(k, min, max, step)`, `gs.checkbox(k, label, invert)`, cards
  `gs.toggle_card(..)`, `gs.dropdown_card(..)` (None when the key is unavailable). Optimistic writes, reverted with
  the error toast; a locked key is insensitive with the tooltip "Sommige van deze instellingen worden beheerd door
  je organisatie."; an owned key becomes a link to the WinPoint page that sets it; they follow `gsettings.changed`.
  Every bound control is named `st-<page>-<key>`, or `st-<page>-<part>-<key>` when the page's `Gs::load` has that
  key name from more than one schema or path (`gs::control_name`): `<part>` is the schema id's last segment
  (`st-proxy-https-host`, `st-multitasking-app-switcher-current-workspace-only`), or for one relocatable schema at
  two paths the path's last segment with `:` as `-` (`st-pen-056a-0358-area`).
* `p.set_title(text)` replaces the title, on a sub-page the last crumb (a data page's item name: "Netwerk en
  internet › Wi-Fi › Thuis"; `tests/gnome/lib/scenario.py` `settings_data_page(..., title)` waits for it).
* `pickers::{TimePicker, DatePicker, KeyCapture (Capture::{Accel, Disabled, Cancelled}), QrCode, admin_button}`.
  KeyCapture sends `input.capture.cancel` when its dialog closes while the host has not answered.
* `common::child_links(&p, parent)` (registry-driven, caps-filtered link cards), `common::placeholder` (the B0
  builder of every page a batch has not built yet), `common::caps()`/`common::caps_ok(needs)`.

### 6.4 Mirror (`host/src/settings_mirror.rs`, panel host only)

Pairs (B0): `notifications.dnd` ↔ `show-banners` (inverted); `time.h24` ↔ `clock-format`; `time.showSeconds` and
`taskbar.showSeconds` ↔ `clock-show-seconds`; `time.showDate` ↔ `clock-show-date`; `time.showWeekday` ↔
`clock-show-weekday`; `time.weekNumbers` ↔ calendar `show-weekdate`; `time.firstDayOfWeek` (-1, 0-6) ↔
`week-start-day`; `privacy.recentFiles` ↔ `remember-recent-files`; `taskbar.batteryPercent` ↔
`show-battery-percentage`. Look pair: `theme.accent` → `accent-color` while `theme.syncSystem`, through
`gnome_look::note`: the nearest of blue, teal, green, yellow, orange, red, pink, purple, slate and brown among the
names the installed key takes (brown is Ubuntu's only), never to a locked key; the record is made only for a write
that happens (type, range and lockdown are checked first). Render pairs (A, `settings_mirror/render.rs`):
textScale, reduceMotion (+ enable-animations), highContrast. Equality guard plus origin tags (the mirror's own
writes, per key in write order: a handler that reads a tagged value consumes that tag and every older one of the
key; a write GNOME did not take leaves no tag) so writers never loop.

Start, at every panel-host start (amendment 47 as refined): `<config>/gnome-mirror.json` (next to settings.json)
holds, per pair, the values both sides last agreed on (`{"pairs": {"<schema> <key>": {"gnome": GVariant text,
"winpoint": {path: value}}}}`; written at the start and after every change the mirror carries, in both directions).
GNOME moved since (or no record: the first start) → GNOME's value goes to settings.json; only settings.json moved →
it goes to GNOME; both moved → GNOME. Then the accent goes settings.json → GNOME, under the settings lock
(`with_snapshot`, as every other `gnome_look::note` caller).

### 6.5 New settings keys (`host/src/settings-defaults.json`)

`time.showWeekday` false, `time.weekNumbers` false, `time.firstDayOfWeek` -1 (was 1; -1 = the region's; the
Datum en tijd dropdown offers "Standaard (regio)" and all seven days, since the mirror brings any day GNOME holds),
`taskbar.batteryPercent` false, `taskbar.allDesktops` false, `taskView.allDesktops` false, `start.windowsKey` true,
`accessibility.highContrast` false.

### 6.6 Shared helpers

* `host/src/services/system/common.rs` `call_interactive(bus, name, path, iface, method, args, lang)`:
  `ALLOW_INTERACTIVE_AUTHORIZATION`, 10 minutes; NotAuthorized/AccessDenied/PermissionDenied → `denied
  core.notAuthorized`, Cancelled/dismissed → `denied core.authCancelled`, other errors → `failed core.callFailed`.
* `host/src/tools.rs` `system_tool(name)`: `/usr/bin/<name>`, or `$WIN11_SYSTEM_TOOL_DIR/<name>` (a private 0700
  directory of ours) in the sandboxes; `libexec(name)` = `/usr/libexec/<name>`. Every new spawn goes through it.
* `host/src/dlib.rs` `dlib!` (the `bindings!` pattern of `files/archive_ffi.rs`, with several sonames): a missing
  library or symbol means that feature reports unavailable.
* `extension/launchBridge.js`: `launch.app {id}` (an allowlisted desktop id, started as a GNOME Shell child),
  `launch.ibusSetup {engine}` (an IBus engine name, `[\w.+-]` parts joined by up to two `:`): as GNOME Settings
  (cc-input-source-ibus.c, cc-input-list-box.c), the desktop file `ibus-setup-<part before ':'>.desktop` through
  `global.create_app_launch_context` with `IBUS_ENGINE_NAME=<engine>`; not installed → `not_found`.
  `displayBridge.js` and `inputBridge.js` are D's and A's stubs with the same shape (`constructor(emit, language)`,
  `enable()`, `disable()`, `call(method, args)`).
* `host/src/gnome.rs` `call_api_with_timeout(method, args, timeout_ms)`: `call_api` with its own D-Bus timeout, for
  a bridge method that waits for the user (`input.capture`); every other call keeps 3 s.

## 7. Tests

Where: host unit tests in `tests/rust-sandbox/run-tests.sh --lib` (keyfile GSettings, private session bus, stubs,
`WIN11_SYSTEM_TOOL_DIR` = its guard-bin); D-Bus fakes in `tests/dbus-fakes/run.sh --batch <b> -- BIN --ignored --exact
<test>` (fakes in `tests/dbus-fakes/fakes/<batch>/`, template `fakes/b0/echo.py`, `--self-test` checks the layer);
harness stand-ins through `tests/gnome/lib/standins`; native unit tests in `cargo test --manifest-path
native/Cargo.toml`; `tests/i18n/test_native.py` for the registry ↔ settings_pages.json, the caps keys and the
windows-glossary files.

B0: allowlist audit (schemas installed or optional, keys exist with a mappable type, relocatable entries through the
schema id, no forbidden schema or key, look keys Owned, uniqueness, no `disable-while-typing-timeout`); JSON ↔
GVariant round trips; memory backend set/read-back/reset/multi-key/multi-schema; keyfile backend with a locks file →
denied; wrong type/range → bad_args; dry run writes nothing; `gsettings.changed` for an external write; held
accelerators from the compiled extension schema; mirror both directions with a temporary settings store and the
memory backend, no loop, accent through `gnome_look::note`; caps on fixture /sys and /run/udev trees;
api-methods (no duplicates, every METHODS name listed with its dangerous flag, the old list unchanged);
`call_interactive` against a private-bus fake (flag and error mapping); `tools::system_tool`; `dlib`; native
registry (every id has a builder, parent chains acyclic and at most three deep, needs known, breadcrumbs, search
order, data pages excluded) and `tests/i18n/test_native.py` (settings_pages.json = registry minus home and data
pages, caps keys = needs keys). Per batch: plan.testing plus the critique's 63-69.

Added by the B0 review: invalid paths (`//`, `/a//b/`, other characters) refused by `gsettings.*`, `read_item`,
`write_values` and `Env::settings` before any settings object, and no settings object at a relocatable path no row
matches; `write_values` refusing a path outside its row's holes, another method's rows, Owned, Read and unlisted
keys; every written key named by a control in the sources; the api-methods tables in both directions; the bridge
call with its own timeout (private-bus fake); caps seeing a tablet whose udev entry comes after its node; the
mirror's accent among the names GNOME takes and never to a locked key, the tags of late handlers and failed writes,
and the start keeping the side that moved; native: the last crumb of a data page, caps that arrive late, the
control names, the first-day options, `kit::i18n::resolve`.

## 8. Integration notes (B0 → I)

These belong to files of the multi-monitor worktree; B0 did not change them.

1. `extension/panelBridge.js`: import `LaunchBridge` (`launchBridge.js`), `DisplayBridge` (`displayBridge.js`) and
   `InputBridge` (`inputBridge.js`); construct each with `((event, data) => this._emit(event, data), this._language)`,
   `enable()`/`disable()` them with the other bridges, and in `_call` route `method.startsWith('launch.')`,
   `'display.'` (bridge names `display.identify`, `display.confirm`) and `'input.'` (bridge names `input.capture`,
   `input.padOsd`) to `bridge.call(method, args)`.
2. `native/shell/src/panel/taskbar/system.rs`: the tray clock shows the weekday when `time.showWeekday`
   (Windows: "di 17-11-2026" style, `kit::format`), and the battery percentage next to the battery glyph when
   `taskbar.batteryPercent`; both follow settings changes.
3. `native/shell/src/panel/**`: `taskbar.allDesktops` (amendment 12): false shows only the current workspace's
   windows on the taskbar.
4. `native/apps/settings/src/pages/taskbar.rs`: the line "De Ubuntu Dock wordt in WinPoint niet gebruikt" / "The
   Ubuntu Dock isn't used in WinPoint" (amendment 27).
5. A's handoff patches: `native/shell/src/desktop/grid.rs` (text scale), `native/shell/src/panel/taskbar/input.rs`
   plus its hook in `taskbar/system.rs` (amendment 15).

Not in the multi-monitor worktree, but left for I because no batch owns them:

6. `native/shell/src/start/search.rs`: Start offers every page of `settings_pages.json`, also one whose `needs` caps
   key is false here (Touchpad on a desktop). I decides whether Start asks `caps.get` (and the JSON gains a `needs`
   field, test_native.py `registry_pages` with it) or Settings' own "not available" page is enough.
7. CHANGELOG (amendment 47 as refined, section 6.4): the mirror runs at every panel start; the first start of 0.9.x
   takes GNOME's values for the shared preferences (the clock, DND, recent files, battery percentage, week numbers,
   first day), later starts keep whichever side changed while the panel was down.
8. The harness cannot lock a key: GNOME's keyfile backend reads its locks from `/etc/glib-2.0/settings/locks`, and the
   host has no `/etc/glib-2.0` to bind into the sandbox. Lockdown is covered by the host unit test
   `services::gsettings::tests::a_locked_key_is_denied` (a keyfile backend with a `defaults-dir`); plan.testing's
   harness step 6 keeps only its "a WinPoint-owned key shows its pointer" half.


## 9. Integrated API notes (0.9.1)

The fragments in `host/src/api-methods/{input,devices,network,system,accounts,apps}.json` describe the final optional
arguments and response fields. Integration adds the following clarifications to sections 5 and 6:

- `privacy.app.setAutostart! {id, on}` changes only the user's autostart entry, with GNOME's desktop-file semantics.
- `notifications.app.set` emits `notifications.changed {what: app, appId, canonical}` after a successful real write.
- `privacy.insights.report {cancel: true}` cancels the single running dry-run report; `privacy.security` also reports
  whether Security Center is installed. `privacy.apps.sandboxed` names apps from the permission store.
- `printers.job.authenticate! {id, username?, domain?, password}` supplies the held job's authentication fields in
  the queue's required order. It and printer discovery with credentials are secret methods.
- `input.screenReader.preferences {}` asks the running Orca's `org.gnome.Orca.Service.ShowPreferences` and reports
  unsupported when no screen reader owns the name. `input.capture` is asynchronous through `CallAsync`, with an
  optional timeout; source-current/activate and pad-OSD requests route through InputBridge.
- Night-light events use `display.nightLight.changed`; changing its temporary on/off state does not rewrite its
  schedule. `sound.changed` carries the current state; `sound.level {kind, name, peak}` identifies input/output.
  `battery.changed` carries the charging state rather than a generic empty notification.
- Shared `Gs::set_values` updates and restores cached related values as one operation. Shared service-side
  `gsettings::reset_values_in` checks the effective defaults, including shortcuts held by the extension, before
  resetting anything. Reset-all skips a held default without masking unrelated errors.
- Secret-method recording uses the same method manifests as `runtime::is_secret`. Private test results retain the
  method and success status, while API payloads are redacted.

Amendment 24 is retained as written: the shared shortcut row appears in Sticky, Filter and Toggle keys. Initial
Alt+Tab adoption and the default application language are unchanged pending the user decisions in the handover.
The optional-daemon, real-hardware and portal limits are recorded in `VALIDATION_SETTINGS.md`; no live-session test,
push or release is implied by a successful private run.
