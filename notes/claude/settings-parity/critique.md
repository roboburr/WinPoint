**Critique of the WinPoint Settings plan: 69 numbered amendments**

Every check was read-only (installed files, binaries, gresources, the repo). Nothing touched the live session. Several of the findings below are verified facts that change what the plan can do.

**A. Completeness and Windows fidelity**

1. **Sound › Alle geluidsapparaten / All sound devices** (new page `sound-devices`, parent `sound`). It lists every card, including cards set to "Niet toestaan" (card profile off). Without it, a device switched off on `sound-device` disappears from "Choose where to play sound" and cannot be switched back on. Windows: Sound › Advanced › All sound devices.

2. **Sound, Invoer section.** Add "Een nieuw invoerapparaat koppelen · Apparaat toevoegen" using `go_with(bluetooth, {add:1})`, as for output.

3. **Sound, startup sound.** Move "Opstartgeluid afspelen" (`org.gnome.shell.ubuntu startup-sound`) off the main Sound page. Put it in the "Meer geluidsinstellingen" dialog, tab Geluiden, under Geluidsschema, where Windows has its startup-sound checkbox.

4. **Notification sounds are broken today, so F's sound plan is wrong (verified).**
   - gnome-shell plays a notification sound only in `MessageTray._updateShowingNotification`, which runs when it shows its own banner (`messageTray.js` around line 1162).
   - `extension/notificationsBridge.js` blocks GNOME banners and hands them to WinPoint. So in WinPoint no notification sound ever plays, and `notifications.sounds` has nothing to drop.
   - Amend F: when the WinPoint consumer shows a toast, the bridge calls `notification.playSound()`, which already checks `policy.enableSound`. It skips the call while `notifications.sounds` is false or DND is on.
   - The WinPoint toast and list must also honour `policy.enable` and `policy.forceExpanded`.
   - Delete the plan's "drop the sound before GNOME plays it" step.

5. **Notifications layout.** Put "Meldingen weergeven op het vergrendelingsscherm" and "Toestaan dat meldingen geluiden afspelen" inside the "Meldingen" expander, as Windows does, not as loose checkboxes.

6. **Per-app notification first write touches two schemas.** It needs `application-children` in `org.gnome.desktop.notifications` plus the relocatable child with `application-id`. `gsettings.set` handles one schema and path per call. Add a dedicated `notifications.app.set` in F that does both, as g-c-c does. Put `application-children` in Read mode in the allowlist.

7. **App › Advanced options corrections.**
   - The PermissionStore id for "Systeemsneltoetsen blokkeren" is `gnome` / `shortcuts-inhibitor` (`inhibitShortcutsDialog.js` `APP_PERMISSIONS_ID`), not `shortcuts-inhibit`.
   - Enable the Autostart row only while Background is allowed, as GNOME does.
   - Add GNOME's empty-state "Zoeken in App Center" button to Installed apps.

8. **Pen page: touchscreen mapping is missing.** Add "Aanraakscherm toewijzen aan beeldscherm" when `caps.touchscreen`. It is the relocatable `org.gnome.desktop.peripherals.touchscreen` at `/org/gnome/desktop/peripherals/touchscreens/<vid>:<pid>/` (key `output`). Both libmutter-18 and g-c-c use this path.

9. **Pen allowlist path patterns are too narrow (verified in libmutter-18 strings).**
   - Styli without a serial use `/org/gnome/desktop/peripherals/stylus/default-<vid>:<pid>/`.
   - Pad controls use `tablets/<vid>:<pid>/<name>/`, which covers rings, strips and dials as well as buttons.
   - `{serial}` must also accept `default-{vidpid}`, and the pad hole must accept those control ids. Otherwise common pens without serials fail.

10. **Touchpad: drop the "Gevoeligheid van touchpad" dropdown.** libmutter-18 contains no `disable-while-typing-timeout` string, so mutter 50 ignores the key. Keep only the "Touchpad uitschakelen tijdens het typen" checkbox.

11. **Multitasking hot corner does nothing in WinPoint.** `extension/desktopBridge.js` `_syncHotCorners` neutralises GNOME hot corners whenever the WinPoint desktop is present.
    - Either make the corner open Taakweergave in WinPoint, or label the row "Geldt in de GNOME- en Ubuntu-sessie".
    - Either way, place it last as a WinPoint addition, not second.

12. **Multitasking app switching is mapped to the wrong Windows rows.**
    - In WinPoint, Alt+Tab is WinPoint Task View. The extension strips `<Alt>Tab` from every `wm.keybindings` key, and `task-view` defaults to `['<Alt>Tab']`. Super+Tab stays GNOME's `switch-applications`.
    - Use Windows' two rows in the Bureaubladen expander, with options "Alleen op het bureaublad dat ik gebruik / Op alle bureaubladen":
      - "Op de taakbalk alle geopende vensters weergeven": a new WinPoint taskbar setting.
      - "Alle geopende vensters weergeven wanneer ik op Alt+Tab druk": a WinPoint Task View setting. It also writes `org.gnome.shell.app-switcher` and `window-switcher current-workspace-only` so GNOME's Super+Tab behaves the same.
    - Order the page as Windows does: Vensters uitlijnen, Bureaubladen, then the WinPoint additions.

13. **Shortcuts page.**
    - Make WinPoint's own keys editable: `org.gnome.shell.extensions.win11-gnome task-manager` and `task-view` (`Main.wm.addKeybinding` follows changes to the key).
    - Check for keys the extension holds per accelerator, not per whole key. The held ones are `<Alt>Tab` in any wm key and Ctrl+Alt+Delete in `media-keys logout`.
    - Refusing all of `switch-applications` would block editing Super+Tab. A write that contains `<Alt>Tab` is harmless: the extension strips it again and updates `task-view-restore`. Show it as the conflict "In gebruik door Taakweergave".

14. **Sticky keys disable the Windows key.** `shortcutBridge.js` ignores the overlay key while `stickykeys-enable` is on. Either say so on `a11y-keyboard` ("Met Plaktoetsen opent de Windows-toets Start niet") or change the bridge.

15. **No input-source indicator exists anywhere in WinPoint.** Nothing in `native/shell` or `extension` references input sources, and the GNOME top bar is hidden.
    - Once `language-options` can add a second layout, add a tray input indicator ("Invoerindicator", the way Windows shows ENG/NLD).
    - A owns it in a new `native/shell/src/panel/taskbar/input.rs`. B0 adds the hook in `taskbar/system.rs`.

16. **Date & time.**
    - Windows keeps the 24-hour format and "Eerste dag van de week" under Taal en regio › Regionale notatie › Notaties wijzigen. Keep them only on `formats`, with a link from `datetime`, instead of on both pages.
    - Order `datetime` as Windows does:
      1. Tijd automatisch instellen
      2. Tijdzone automatisch instellen
      3. De datum en tijd handmatig instellen
      4. Tijdzone
      5. Automatisch aanpassen aan zomertijd (shown on and disabled)
      6. The system-tray toggles
      7. Aanvullende instellingen: Nu synchroniseren

17. **Regional format.** Windows shows it inline on Taal en regio: a dropdown plus an expander with samples and a "Notaties wijzigen" button. `formats` should be the sub-page that button opens, not the main entry.

18. **One display language, not two.** Two "display language" dropdowns on one page is not Windows.
    - The "Weergavetaal" row should be the system language (AccountsService `SetLanguage`, with the sign-out banner).
    - WinPoint's nl/en follows it by default, with a "Taal van WinPoint" override row beneath.
    - The `language` setting gains the string value `"system"`, which stays type-valid.

19. **Display layout.**
    - The presentation-mode dropdown (Dupliceren / Uitbreiden / Alleen weergeven op N) belongs beside Identificeren in the arrangement card.
    - The "Meerdere beeldschermen" expander closes "Schaal en indeling". It holds "Dit mijn hoofdbeeldscherm maken" and "Ander beeldscherm detecteren".
    - Kleurprofiel is a row in "Helderheid en kleur" (24H2), not a related link.
    - Related settings: Geavanceerd beeldscherm.

20. **Power & battery layout.**
    - Put the power-button dropdown inside the expander "Besturingselementen voor deksel en aan/uit-knop", as one column because GNOME has one value. Lid rows are omitted.
    - Put both energy-saver rows inside the "Energiebesparing" expander.
    - Put "Batterij opladen" and "Batterijniveaus" in the Batterij section.
    - Add GNOME's warning when automatic suspend is set to Nooit.

21. **Storage Sense.**
    - Use Windows' trash presets: Nooit, 1 dag, 14 dagen, 30 dagen, 60 dagen. `old-files-age` (type u) accepts any value. Keep a non-preset current value as an extra option, and keep "1 uur" (value 0) as a GNOME extra.
    - The master switch sets both `remove-old-trash-files` and `remove-old-temp-files` (on = both true, off = both false). Its state is "either is on".
    - Add Windows' single "Opslagzin nu uitvoeren" button. The two explicit buttons stay below it.

22. **Default apps should follow Windows 11 more closely.** Windows 11 has no category dropdowns at the top. Add:
    - the search "Een standaardwaarde instellen voor een bestandstype of koppelingstype";
    - the list "Standaardwaarden instellen voor toepassingen", whose app page shows GNOME's Files & Links (reuse the builder from `app-advanced`'s handlers section);
    - "Alle standaard-apps opnieuw instellen".

23. **Sign-in options sentence.** Use Windows' wording "Als je weg bent geweest, wanneer moet Windows vereisen dat je je opnieuw aanmeldt?", with the same Windows → WinPoint substitution used elsewhere. Place it under "Aanvullende instellingen".

24. **Accessibility › Keyboard.** Windows shows "Sneltoets voor Plaktoetsen / Filtertoetsen / Wisseltoetsen" inside each expander. GNOME has a single key (`a11y.keyboard enable`). Show the row in all three expanders, bound to that key, with a note that it covers all three. Drop the separate row.

25. **Night light "Nu inschakelen".** Outside the schedule, the plan silently rewrites the user's schedule. Instead:
    - Offer "Nu uitschakelen" (sets `DisabledUntilTomorrow`).
    - Offer "Nu inschakelen" only while `DisabledUntilTomorrow` is true.
    - Outside the schedule, show info such as "Wordt om 20:00 ingeschakeld".

26. **Data pages need a registry flag.** `wifi-network`, `printer`, `app-advanced`, `app-notifications` and `sound-device` need a param, so flag them in the registry. Exclude them from search, from Start's `settings_pages.json`, from Home's recent list and from aliases.

27. **Give the left-out Ubuntu Desktop panel a visible place.** Add one line on Taakbalk and on Bureaubladpictogrammen: "De Ubuntu Dock / de bureaubladpictogrammen van Ubuntu worden in WinPoint niet gebruikt". Robin asked for every function to have a place.

28. **Add a wording gate.**
    - Many nl strings come from the unverified Windows inventory, and some contradict existing ones. For example, `sound.rs` has "Kies waar je geluid wilt afspelen" while the plan has "Kies waar geluid moet worden afgespeeld". Other examples: "Verbinding met een datalimiet" against the existing "verbinding met datalimiet", and "Afdrukstand van beeldscherm".
    - No Dutch g-c-c catalogue is installed here (only en_GB), so "check against nl.po" is not possible locally.
    - B0 seeds `tests/i18n/windows-glossary.tsv` (en, nl, source, verified). Batches take their strings from it, and step I fails on new UI strings that are missing from it.
    - Existing strings stay until a verified Windows string replaces them.

29. **Record the deliberate renames.** "Pen en tablet" (Windows: "Pen en Windows Ink"), "Schermtijd" (no Windows page) and "Deze pc doorzoeken" ("Windows doorzoeken") are deliberate. Record them in that glossary.

**B. Feasibility and architecture**

30. **There is no `timesync1` (verified).**
    - chrony 4.8 is installed and systemd-timesyncd is not, so `org.freedesktop.timesync1` does not exist.
    - "Nu synchroniseren" becomes timedate1 `SetNTP(false)` then `SetNTP(true)`. This restarts `chrony.service`, which is listed in `/usr/lib/systemd/ntp-units.d/50-chrony.list`.
    - Status comes from timedate1 `NTPSynchronized`. The server name comes from chrony's sources files (read-only), or the row is hidden.

31. **Brightness is owned by mutter in GNOME 50.**
    - libmutter-18 has `MetaBacklight` and DisplayConfig `SetBacklight` / `Backlight`, with polkit `org.gnome.mutter.backlight-helper`. gsd-power dims and auto-adjusts through `org.gnome.Shell.Brightness` (`SetDimming`, `SetAutoBrightnessTarget`).
    - logind `SetBrightness` bypasses mutter's level, so idle dim and undim and the shell work from a stale value.
    - D: move `system.brightness.*` to DisplayConfig `SetBacklight` and the `Backlight` property. Keep logind for Hyprland and as a fallback.

32. **Display confirmation: avoid two dialogs (verified).**
    - gnome-shell opens its own `DisplayChangeDialog` on mutter's `confirm-display-change` signal (`windowManager.js`), and the handler is bound at construction.
    - Do not use temporary-then-persistent applies with a second WinPoint dialog.
    - Instead, `displayBridge.js` blocks the shell's handler (`signal_handler_find` / `block`, as `shortcutBridge` does for `overlay-key`). It relays `display.confirmChange` to Settings, which shows the Windows 15 s dialog, and answers with `global.window_manager.complete_display_change(ok)`. The shell's dialog stays as the fallback when no consumer is attached.

33. **DisplayConfig signatures.** `ApplyMonitorsConfig` `a(iiduba(ssa{sv}))` and `GetCurrentState` `a((ssss)a(siiddada{sv})a{sv})` appear in libmutter-18. No DisplayConfig XML is installed, so tests use fixtures rather than an interface file.

34. **RDP certificate needs gnutls.**
    - g-c-c generates the RDP TLS certificate in-process with gnutls (`bonsai_tls_certificate_*`).
    - grdctl 50 has `set-credentials`, `set-port` and similar commands, but nothing that generates a certificate.
    - Add `libgnutls.so.30` to the dlopen list (it is already loaded by libcups), or Desktop Sharing cannot be enabled on a fresh account.

35. **SSH uses socket activation on Ubuntu.**
    - g-c-c knows both `ssh.service` and `ssh.socket`. `sharing.ssh.*` must enable and start `ssh.socket` when present, and stop both.
    - Polkit is `systemd1.manage-units` / `manage-unit-files` (auth_admin_keep). Ubuntu has no YES rule for these, so expect a password prompt.

36. **Orca preferences.** GNOME 50 uses D-Bus `org.gnome.Orca.Service` `/org/gnome/Orca/Service` `ShowPreferences` on the running Orca (`orca/dbus_service.py`). Call it from the host and drop `launchBridge` for Orca.

37. **GNOME version.** `/usr/share/gnome/gnome-version.xml` is not installed, and g-c-c 50 does not reference it. Read `org.gnome.Shell ShellVersion` instead.

38. **Ubuntu insights.**
    - g-c-c uses `libinsights.so.0` (package libinsights0, a Go cgo library) for consent and compile.
    - Do not dlopen a Go runtime into the host, and do not hand-write the consent TOML.
    - Use `/usr/bin/ubuntu-insights` (consent; collect dry-run) through `system_tool`, and verify its subcommands in F.
    - WhoopsiePreferences over D-Bus is fine; Ubuntu's rule returns YES for sudo users.

39. **Sound FFI and one source of truth.**
    - Use `libpulse-mainloop-glib.so.0` (installed; g-c-c links it) on the sound service's own GLib MainContext, instead of a `pa_threaded_mainloop`.
    - Keep one source of truth: `system.audio.*` (used by `quick.rs`, `taskbar/system.rs` and `sound.rs`) delegates to `sound.*` when libpulse loads. wpctl is the fallback only, and there is a single changed event.

40. **Spike libnma before hand-writing network forms.**
    - libnma-gtk4 exports `nma_ws_*` (since 1.8.28) and `nma_wifi_dialog_new_for_hidden`.
    - The GTK 4 editors `libnm-gtk4-vpn-plugin-{openvpn,pptp}-editor.so` are installed and need only GTK 4, libnma and libnm (no libadwaita).
    - Before C hand-writes EAP forms and the vpn.data name/value rows, spike embedding them in Settings through dlopen. The host passes the connection as `a{sa{sv}}` (`nm_simple_connection_new_from_dbus` / `nm_connection_to_dbus`).
    - If it works, every OpenVPN, PPTP and EAP option is covered. If not, keep the plan and record the gap.

41. **Printers.**
    - Verify how a driverless (IPP Everywhere) queue is created through cups-pk-helper before B starts. g-c-c relies on the Python `GetBestDrivers` service that the plan drops.
    - `cupspkhelper.*` is YES for sudo/lpadmin users (`com.ubuntu.desktop.rules`); otherwise it is auth_admin_keep.
    - SMB browsing needs libsmbclient, which g-c-c links. Add it to the dlopen list, or support only a typed `smb://` address.

42. **snapd interactive header.** Snapd writes must send `X-Allow-Interaction: true`, or snapd will not start the polkit prompt (`io.snapcraft.snapd.manage-interfaces`).

43. **Global shortcuts.**
    - `org.gnome.GlobalShortcutsRebind` is served by xdg-desktop-portal-gnome at `/org/gnome/globalshortcuts`, not by g-c-c's provider. Pin the bus name and path.
    - `global-shortcuts.application shortcuts` is `a(sa{sv})`, which the gsettings value mapper refuses. Read it natively in `privacy.app`.

44. **KeyCapture.**
    - GTK's inhibit-system-shortcuts makes gnome-shell ask for permission for every app except `org.gnome.Settings.desktop` (`APP_ALLOWLIST`).
    - Capture in `inputBridge.js` instead: `Main.pushModal`, turn the next key event into an accelerator, release. No prompt and no PermissionStore write.

45. **Searching this PC.**
    - The Tracker keys affect only localsearch and Nautilus. WinPoint Start uses its own index (`index.setRoots`).
    - The page drives `index.setRoots` first and mirrors the same folders into the `Tracker3.Miner.Files` keys (localsearch 3.11 keeps that schema).
    - Search providers change GNOME's overview only. Say so on the page, or let Start query `ShellSearchProvider2`.

46. **High contrast.**
    - libadwaita apps take high contrast from the portal, but WinPoint's Fluent import block in `~/.config/gtk-4.0/gtk.css` overrides it in the WinPoint session.
    - While `high-contrast` is true, `settings_effects` drops the Fluent block, and the kit honours `@media (prefers-contrast: more)`.
    - A owns this, through a hook B0 adds.

47. **Mirror direction at startup.**
    - The mirror lives only in the panel host. Changes made in a GNOME or Ubuntu session, or while the panel is down, leave `settings.json` stale.
    - On panel start: GNOME → `settings.json` for every preference pair; `settings.json` → GNOME only for the accent look pair.
    - Otherwise the first WinPoint start after the update overwrites GNOME values, for example a textScale of 125 pushes `text-scaling-factor` to 1.25 in every session.
    - Note the one-time adoption in the CHANGELOG.

48. **Mirror value domains.**
    - `settings.rs` validates values by JSON type; only `alles.roots` has an exception. So `time.firstDayOfWeek` cannot hold both `'default'` and 0-6: use -1 for "Standaard (regio)".
    - `textScale` is 100-225 while `text-scaling-factor` is 0.5-3.0. Never clamp a GNOME value and write it back. Widen the range to 50-300, or mirror out-of-range values one way only.

49. **Which side writes each mirrored pair.**
    - Settings pages write the GNOME key; the allowlist mode is `Mirrored(setting)`, and the mirror updates `settings.json`.
    - Other WinPoint UIs (DND flyout, Start) keep writing `settings.json`, and the mirror writes GNOME.
    - The textScale, reduceMotion and highContrast pairs ship with A, not B0, so main never double-scales.

50. **Allowlist uniqueness.** B0 puts every (schema, key) in exactly one allow table, with a test. Assignments:
    - `allow-volume-above-100-percent` (used by D and A): one owner.
    - `locate-pointer`, `double-click`, `screen-keyboard-enabled`: A.
    - `show-in-lock-screen`: F.
    - `idle-delay` and `sleep-inactive-*`: Owned by `system.power.settings`; `energy.rs` stays the only writer.
    - `input-sources sources`: Read, so writes go through `input.sources.set` and the Debian keyboard sync runs.
    - `application-children`: Read.

51. **Multi-schema atomic writes.** Proxy Save touches `org.gnome.system.proxy` plus `.http`, `.https`, `.ftp` and `.socks`. Either `gsettings.set` accepts a list of `{schema, path, values}`, validates all of them first and writes `mode` last, or C adds a dedicated `net.proxy.set`.

52. **Polkit facts to record in the plan (verified).**
    - Ubuntu rules return YES for local active sudo users on:
      - `timedate1.set-*`
      - `hostname1` `set-hostname` and `set-static-hostname`
      - `locale1` `set-locale` and `set-keyboard`
      - `controlcenter.set-keyboard`
      - ModemManager `Device.Control` and `SetPrimarySimSlot`
      - NM `settings.modify.system` (sudo or netdev)
      - `cupspkhelper.*` (sudo or lpadmin)
      - `whoopsiepreferences.change`
    - Everything else prompts:
      - `accounts.user-administration` and `set-login-option`
      - `systemd1.manage-units`
      - `bolt.manage`
      - colord `install-system-wide`
      - snapd
      - PackageKit install
      - the gnome-remote-desktop daemon
      - fwupd `fix-host-security-attr` (auth_admin, no keep)
    - `set-keyboard` and `enable-system-daemon` carry `exec.path` annotations, so `pkexec <helper>` is correct.
    - The `set-debian-keyboard` argv is confirmed as `<layouts> <variants> [options]`, with characters limited to `[a-zA-Z0-9,_:-]`. It runs dpkg-reconfigure (slow), so run it asynchronously after the GSettings write.

53. **Secrets travel over the HTTP API as plain text.** These methods carry them:
    - `users.setPassword`, `users.changePassword`, `users.create`
    - `net` secrets and `net.wifi.secrets`
    - `sharing.rdp.credentials`
    - goa
    - modem PIN/PUK
    - the existing Wi-Fi connect password

    Mark them `"secret": true` in the api-methods fragments. The runtime never logs their args, error data or dry-run toasts, and the harness redacts them.

54. **Existing drift B0 must fix instead of "keeping /api/methods identical".** `system.power.settings.get` and `.set` are dispatched by `system.rs` but are missing from `api-methods.json`. Today the power page's writes get no dryRun envelope, and B0's own "every METHODS name is listed" test will fail until they are added.

55. **Shared entry points missing from B0's file list.**
    - `host/src/events.rs` `valid_name` prefix list. Without these prefixes their events are dropped: `gsettings.`, `caps.`, `input.`, `devices.`, `printers.`, `net.`, `sharing.`, `sound.`, `battery.`, `wellbeing.`, `users.`, `goa.`, `machine.`, `privacy.`, `background.`
    - `host/src/gnome.rs` `relays_event` (`display.confirmChange` and the `input.*` bridge events).
    - The `tests/i18n` allow lists for the new stems.

56. **Session kind.** Hyprland code paths remain (`display.rs`, `power.rs`, `quick.rs`). `caps.get` reports the session (gnome, hyprland or other), and GNOME-only pages declare `needs: "gnome"`.

57. **Flight mode and Bluetooth radios.** GNOME uses gsd Rfkill (`AirplaneMode`, `BluetoothAirplaneMode` on the session bus). Route `flightmode` and Bluetooth on/off through it when present, keeping the nmcli/rfkill path as fallback, so the state matches GNOME's after the next login.

58. **The Windows key.** `shortcutBridge.js` opens Start from mutter's `overlay-key` signal, so `overlay-key ''` also kills Start, and the change applies to GNOME and Ubuntu sessions too. The confirm dialog must say both. Better: make "Windows-toets opent Start" a WinPoint setting read by the bridge, and keep GNOME's key as a separate row marked GNOME-only.

59. **Locales.**
    - `machine.locales` lists generated locales (`localedef --list-archive` through `system_tool`), as gnome-desktop does.
    - `language-pack-gnome-nl` is not installed here, so `SetLanguage nl_NL` is only useful after `languages.install`. Order the UI accordingly.

**C. Batches and tests**

60. **Split B0** to shorten the critical path and lower its risk.
    - **B0a, scaffolding:** registry, ALIASES, stubs, `pages/mod.rs`, `services/mod.rs`, `events.rs`, `gnome.rs` relay, api fragments and loader, i18n stems, `panelBridge` dispatch, harness stubs, `child_links`, breadcrumbs.
    - **B0b, foundation:** the gsettings service and allow tables, `gs.rs`, `pickers.rs`, `caps`, `call_interactive`, `system_tool`, `dlib`.
    - **M, the mirror** (after B0b, parallel with A-F): owns `settings_mirror.rs`, `settings-defaults.json`, `taskbar/system.rs` and the calendar consumers. The three rendering pairs go to A.

61. **Ownership fixes.**
    - (a) `display.tsv`, `storage.tsv`, `notifications.tsv` and `apps.tsv` already exist. B0 must not recreate them as empty files.
    - (b) Every batch adds D-Bus stand-ins, so a single `tests/gnome/lib/standins.py` will conflict. Make it a builder, with `tests/gnome/lib/standins/<batch>.py` per batch.
    - (c) The harness stub list is missing tools the batches spawn: `nmcli` import, `pkexec` logging for `set-debian-keyboard` and `gnome-remote-desktop-enable-service`, `localedef`, `ubuntu-insights`, `snap-store` / `gnome-software`, `update-manager`.
    - (d) `home.rs` has no owner but calls `bluetooth.rs` helpers (B) and `system::device_hero` / `rename_hint` (E). Freeze those signatures in B0 with a compile test.
    - (e) A's kit changes reach every native app. Keep them additive, and land them first within A.

62. **Sound needs a real audio stack in the harness.** The harness buses run no PipeWire. Start pipewire, wireplumber and pipewire-pulse (all installed) inside the harness for `settings-system.py`, or the sound pages are covered by unit tests only. Also state in `VALIDATION_SETTINGS.md` that the harness checks keys, not gsd effects.

63. **Test for item 4.** With per-app `enable-sound-alerts` true, a GNOME notification plays exactly one sound while the WinPoint toast shows. It plays none when `notifications.sounds` is false or DND is on.

64. **Test for item 32.** A persistent apply shows exactly one keep-changes dialog. On timeout the previous configuration comes back.

65. **Registry tests.**
    - Data pages are excluded from search, Start and recent lists.
    - Every `needs` value is a known caps key.
    - nl "muis" and en "mouse" find `mouse` before `a11y-mouse`.

66. **Integration step I additions.**
    - `win11-app --gnome-look` restores accent-color.
    - Nothing any batch wrote landed in the ubuntu-dock, ding or tiling-assistant schemas.
    - `org.gnome.shell enabled-extensions` and mutter `experimental-features` are unchanged.

67. **Test for item 10.** The touchpad page has no sensitivity dropdown, and a test asserts that `disable-while-typing-timeout` is not in the allowlist. It stays out unless a later mutter reads it.

68. **Test for item 30.** On a machine without timesync1, Date & time shows no error toast and "Nu synchroniseren" calls only timedate1 `SetNTP`. Check this in `settings-time.py` with the stand-in.

69. **The 15 s keep dialog moves from D's Settings code to the bridge path of item 32.** Hyprland keeps its existing `keep_changes_dialog`. The two paths must share the same nl/en strings ("Deze beeldscherminstellingen behouden?", "Wijzigingen behouden", "Terugzetten").
