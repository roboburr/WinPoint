# Settings parity, batch D (System): handoff

Contract: notes/claude/settings-parity.md (§1 amendments, §2 pages, §4.5 files, §5.5 host API). Batch D runs in three
slices; each appends its own section here.

## Slice 1: Beeldscherm, Geavanceerd beeldscherm, Nachtlampje

### What was built

Host (`host/src/services/display.rs`, `display/mutter.rs`, `display/fake.rs` (tests only), `display/tests.rs`,
`services/system/hardware.rs`; `api-methods/system.json`; `host/i18n/display.tsv`):

* `display.state {}`: Mutter's `GetCurrentState` decoded (monitors with their modes, supported scales, preferred
  and current mode, VRR modes (`refresh-rate-mode` variable), underscanning when supported, privacy screen, colour
  modes, rgb range, the logical monitors) plus the DisplayConfig properties `ApplyMonitorsConfigAllowed`,
  `NightLightSupported` and `PanelOrientationManaged`. Without DisplayConfig: `{supported: false, backend:
  'hyprland' | 'none'}` (the page then keeps display.list / display.set).
* `display.apply! {serial, method, logical, props?}`: checked first (`bad_args display.badConfig`), then
  `ApplyMonitorsConfig` `(uua(iiduba(ssa{sv}))a{sv})`. A persistent change is verified (method 0) first, then the
  bridge is asked to hold back GNOME Shell's own dialog (`display.expectConfirm`), then applied persistently
  (method 2). The answer says who asks: `confirm: 'settings'` (with `id` and `timeout` 15) or `'shell'` (the
  bridge was not there: GNOME Shell's DisplayChangeDialog asks, as GNOME Settings relies on). A refused persistent
  call releases the hold (`display.release`). Errors: `display.stale` (serial), `display.notAllowed`,
  `display.applyFailed {error}` (mutter's message), `display.unavailable`.
* `display.identify {}`: display.state's numbers to `display.identify {labels, seconds: 3}` of the bridge
  (Main.osdMonitorLabeler, GNOME Settings' ShowMonitorLabels is limited to its own bus name);
  `unsupported display.identifyUnavailable` without the bridge (the page then flashes its own tiles).
* `display.confirm! {keep, id?}`: the answer through the bridge (`global.window_manager.complete_display_change`).
* `display.nightLight.status {}`: gsd-color's NightLightActive, DisabledUntilTomorrow, Temperature, Sunrise, Sunset
  (`available`: gsd-color answers) and `supported` = mutter's NightLightSupported (GNOME Settings 50's own test for
  "Night Light Unavailable"; an older mutter without the property counts as supported).
* `display.nightLight.disableUntilTomorrow! {on}`: gsd-color's DisabledUntilTomorrow (amendment 25), the schedule
  untouched; answers the new status and emits `display.nightLight.changed`.
* Events: `display.changed {what: 'monitors'}` (MonitorsChanged), `{what: 'properties'}` (one of the three
  properties above changed), `display.nightLight.changed` (gsd-color's PropertiesChanged); the watcher runs on the
  Display service's own GLib context when the session is not Hyprland. Hyprland's `display.changed {monitors}`
  poll is unchanged.
* `system.brightness.*` (amendment 31): `hardware.rs` reads DisplayConfig's `Backlight` property `(uaa{sv})`
  (`connector`, `active`, `min`, `max`, `value`, read from libmutter-18's builder) and sets it with
  `SetBacklight(u serial, s connector, i value)` (one retry after a stale serial); logind `SetBrightness` stays for
  Hyprland and as the fallback when mutter has no backlight. Same payload `{available, value, device}`.

Extension (`extension/displayBridge.js`): `display.identify`, `display.expectConfirm`, `display.release`,
`display.confirm`; event `display.confirmChange {timeout, id}` and `display.identified {shown, connectors?}`.
GNOME Shell's handler (windowManager.js `_confirmDisplayChange`, bound at startup on `global.window_manager`) is
found with `signal_handler_find` before ours connects and blocked only between `expectConfirm` and the next
`confirm-display-change` (at most 5 s), so Super+P, GNOME Settings and every other client keep the shell's dialog.
Mutter's own 20 s timeout reverts when nobody answers (shell's dialog comment: "mutter already takes care of
failing at timeout").

Native (`pages/display.rs`, `pages/nightlight.rs`):

* Beeldscherm (GNOME path when display.state names mutter): the arrangement card (more than one usable monitor):
  numbered tiles to scale, a click selects, a drag snaps to another monitor's edge (24 px edge alignment, never
  overlapping) and applies; Identificeren and the presentation drop-down beside it (Deze beeldschermen dupliceren /
  uitbreiden / Alleen weergeven op N; dupliceren is disabled without a common size). Helderheid en kleur:
  Helderheid (mutter; an expander with "Helderheid automatisch wijzigen wanneer de verlichting verandert" =
  ambient-enabled when caps.ambientLight), Nachtlampje (night-light-enabled switch + chevron to the page),
  Privacyscherm (privacy-screen, when a monitor reports privacy-screen-state or caps.privacyScreen), HDR gebruiken
  (color-mode 1/0 for a monitor offering BT.2100), Kleurprofiel (link, amendment 19). Schaal en indeling (per
  selected monitor, "· Beeldscherm N" with several): Schaal (mutter's supported scales in Windows' 25 % steps plus
  the current and the recommended = preferred one) with Aangepaste schaal (100-500 %, the nearest supported scale),
  Beeldschermresolutie (Aanbevolen = the preferred mode's size; the new mode keeps the refresh rate and the scale
  when it can), Afdrukstand van beeldscherm (transform 0, 1, 2, 3; insensitive for a built-in panel while
  PanelOrientationManaged), Draaivergrendeling (orientation-lock, when caps.accelerometer), and the Meerdere
  beeldschermen expander that closes the group (Dit mijn hoofdbeeldscherm maken, Ander beeldscherm detecteren ·
  Detecteren). Gerelateerde instellingen: Geavanceerd beeldscherm. "Beeldscherminstellingen kunnen nu niet worden
  gewijzigd." with every monitor control insensitive while ApplyMonitorsConfigAllowed is false. Every layout
  change carries the monitors' current underscanning and colour mode (mutter resets what a layout leaves out) and
  is placed again after a size change (a monitor that touched another's edge keeps touching it).
* The keep question (amendments 32, 64, 69): Settings shows `keep_changes_dialog` (the one function Hyprland uses
  too, same strings) for `display.confirmChange` and for an apply answered with `confirm: 'settings'`, once per id
  (both name the same id), and answers `display.confirm {keep, id}`; Terugzetten or the end of the 15 s answer
  keep false.
* Geavanceerd beeldscherm: Selecteer een beeldscherm (Beeldscherm N: name), Beeldscherminformatie (Bureaubladmodus
  = the logical size, Actieve signaalmodus, Schaal, Fabrikant, Model, Serienummer, Kleurruimte SDR/HDR, Kopiëren),
  Een vernieuwingsfrequentie kiezen (the fixed rates of the current size), Dynamische vernieuwingsfrequentie (the
  size's variable mode), Aanpassen voor tv (underscanning, when supported), and "Schaal voor apps herstellen"
  (below).
* Nachtlampje: the status row (Nachtlampje is ingeschakeld · Nu uitschakelen; Uitgeschakeld tot morgen · Nu
  inschakelen; outside the schedule "Wordt om 20:00 ingeschakeld" or "Wordt bij zonsondergang ingeschakeld";
  Nachtlampje is uitgeschakeld), Sterkte (night-light-temperature = 4700 - 30 × strength), Nachtlampje plannen
  (night-light-enabled, expander) with Zonsondergang tot zonsopkomst (hh:mm; hh:mm) / Uren instellen
  (night-light-schedule-automatic), Inschakelen / Uitschakelen (night-light-schedule-from / -to through
  pickers::TimePicker, 24 h per time.h24), the location hint with a link to Locatie when location services are off
  and the schedule is sunset to sunrise; GNOME's "Night Light Unavailable" when mutter says NightLightSupported
  false.
* The Hyprland path is the old page (display.list / display.set, system.nightLight.*, keep_changes_dialog), now
  `hypr_display`; it also keeps the D-G2 notices for a session with neither backend (the host tests compare them).

### Plan controls changed or dropped (with the reason)

* display "Helderheid": DisplayConfig SetBacklight instead of logind (amendment 31).
* display "Schaal": the drop-down offers mutter's supported scales of the mode (Windows' 25 % steps, the current and
  the preferred scale); Aangepaste schaal is a row inside the Schaal expander (Windows has a sub-page), rounded to
  the nearest supported scale. `experimental-features` is read only (it decides nothing here: mutter's supported
  scales already reflect scale-monitor-framebuffer).
* display "HDR gebruiken": a switch row in Helderheid en kleur (Windows has an HDR sub-page); its "SDR content
  brightness" slider is left out (mutter's `output-luminance` has no control in plan.json).
* display "Deze beeldscherminstellingen behouden?": not temporary-then-persistent (amendment 32 rejects that):
  persistent after a verify, the bridge relays the shell's request (amendments 32, 64, 69).
* display related links: Tekstgrootte dropped, Kleurprofiel moved into Helderheid en kleur (amendment 19).
* display-advanced "Wazige apps herstellen": titled "Schaal voor apps herstellen" / "Fix scaling for apps" (the
  windows.json nl), on the advanced page as the plan says. The mapping was read from GNOME Settings 50 itself
  (`/usr/bin/gnome-control-center`, cc-display-panel's `g_settings_bind_with_mapping` on xwayland-scaling-factor:
  active ⇔ |value| < FLT_EPSILON, set active → 0.0, inactive → 1.0) and it shows GNOME's toast "Start apps opnieuw
  om de nieuwe schaal toe te passen". The row shows only while `xwayland-native-scaling` is in mutter's
  experimental-features (without it the key does nothing).
* display-advanced "Beeldscherminformatie": Windows' bit depth and colour format are not reported by mutter; the
  rows are the ones DisplayConfig has (plus Fabrikant, Model, Serienummer, Schaal).
* nightlight "Nu inschakelen / Nu uitschakelen": amendment 25 (no schedule rewrite). With Nachtlampje plannen off
  there is no button (GNOME has no "on now" without a schedule); the row says so.
* nightlight "Nachtlampje is niet beschikbaar": mutter's NightLightSupported decides (GNOME 50); without gsd-color
  the keys still work, only the status row has no active state.
* Left out (Windows only, windows.json says omit): "Helderheid wijzigen op basis van inhoud", "Verbinding maken met
  een draadloos beeldscherm", "Vensterlocaties onthouden ...", "Vensters minimaliseren ...", "Cursorbeweging tussen
  beeldschermen vereenvoudigen". gsd-color's NightLightPreview (GNOME's 5 s preview while dragging the slider) is
  not used: Windows has no preview.

### Notes for I

1. `extension/panelBridge.js` (section 8.1, now with all four bridge names): import and route `display.*`.

   ```js
   import {DisplayBridge} from './displayBridge.js';
   // with the other bridges, after this._emit and this._language exist:
   this._displayBridge = new DisplayBridge((event, data) => this._emit(event, data), this._language);
   this._displayBridge.enable();
   // in disable(), with the other bridges:
   this._displayBridge?.disable();
   this._displayBridge = null;
   // in _call(method, args), before the windows.* switch:
   if (method.startsWith('display.'))
       return this._displayBridge.call(method, args);   // display.identify, .expectConfirm, .release, .confirm
   ```

   `enable()` must run after GNOME Shell's WindowManager exists (it does once extensions are enabled): it looks up
   the shell's `confirm-display-change` handler then. Until this line lands, display.apply answers
   `confirm: 'shell'` and GNOME Shell's own dialog asks (one dialog either way).
2. New event name `display.nightLight.changed` (prefix `display.` is already valid in events.rs); it is the host's
   own (not relayed), like `display.changed`.
3. Registry (optional keywords): display + "hdr", "privacyscherm"/"privacy screen", "draaivergrendeling"/"rotation
   lock", "dupliceren"/"duplicate", "uitbreiden"/"extend", "hoofdbeeldscherm"/"main display"; display-advanced +
   "schaal voor apps herstellen", "dynamische vernieuwingsfrequentie"/"dynamic refresh rate", "vrr"; nightlight +
   "kleurtemperatuur"/"colour temperature", "nu uitschakelen"/"turn off now".
4. `tests/gnome/scenarios/settings-system.py`'s keep check accepts GNOME Shell's dialog until note 1 lands; after
   it, `confirm` must be 'settings' and only Settings' dialog may show (amendment 64).

### Allow rows, i18n, tests

* Allow rows: every key these pages read or write already has its row (B0 put them in `allow/system.rs`:
  night-light-*, ambient-enabled, privacy-screen, orientation-lock, xwayland-scaling-factor Write;
  night-light-last-coordinates and mutter experimental-features Read). Read only from other tables:
  `org.gnome.system.location enabled` (F's `allow/apps.rs`). No row was added or changed.
* Host texts: nine `display.*` rows in `host/i18n/display.tsv` (`#@ new`, listed in
  `tests/i18n/BASELINE-CHANGES.rust-settings-d`, plus the merged `'failed'` literal of hardware.rs); UI strings in
  `tests/i18n/windows-glossary/d.tsv` (glossary rows verified yes, the windows.json ones verified no, WinPoint's
  own wording `winpoint`/no). `keep_changes_dialog` keeps its name: glossary-context.tsv's "Terugzetten = Revert"
  row is scoped to it.
* Host unit tests (tests/rust-sandbox, `--lib`): 585 passed, 19 ignored (604 tests; this slice adds 11:
  `services::display::tests::*` 9, `services::system::hardware::tests::*` 2). The D-Bus side runs against
  `display/fake.rs`, a fake DisplayConfig + gsd-color registered on the sandbox's private session bus under its
  unique name (no well-known names, so tests run in parallel), with GDBus checking mutter's argument signatures
  from the fake's introspection: ApplyMonitorsConfig calls recorded (verify then persistent, the serial, the
  monitor properties), SetBacklight with a stale serial, DisabledUntilTomorrow writes, the dry run writing nothing
  and asking nothing. These are ordinary `--lib` tests (no `tests/dbus-fakes/fakes/d/` fake was needed: every
  call of this slice is on the session bus and none is interactive).
* Native unit tests (`cargo test --release`, native/): 68 passed, 1 ignored; the settings crate 33 passed (this
  slice: 7 in `pages::display::gnome_tests`, 2 in `pages::nightlight::tests`).
* extension/displayBridge.js: `node --check` as a module; its hold / release / confirm / identify logic was run
  against mocked GLib, GObject and Main (a throwaway script outside the repository: one dialog per held change,
  the shell's handler back after the change and after the 5 s hold, labels hidden after their seconds).
* Harness (`tests/gnome/scenarios/settings-system.py`, results in `settings-system.out/` (nl: 18 checks, 0 failed)
  and `settings-system.en.out/` (en: 20 checks with the no-Dutch ones, 0 failed)): the display page takes the GNOME path on the harness's real mutter, every control is
  there, no placeholder, and amendment 64 holds on the real mutter: a persistent display.apply with another scale
  gets exactly one keep dialog (GNOME Shell's, since panelBridge.js does not route `display.*` yet; Settings shows
  none) and, unanswered, the previous scale is back after mutter's 20 s. The harness's virtual monitor reports
  NightLightSupported false, so Nachtlampje shows GNOME's "Night Light Unavailable" there and its controls are
  covered by the unit tests only.

### Open issues

* Settings' own keep dialog only appears after note 1 (panelBridge.js routing) is applied; the harness keep check
  must then expect `confirm: 'settings'`.
* Nachtlampje's controls could not be clicked in the harness (virtual monitor without gamma).
* Dragging in an arrangement of three or more monitors can leave one monitor not touching any other after the
  snap; mutter's verify then refuses it (the error toast) and the arrangement is drawn again from the state.
* Brightness follows the first active backlight in mutter's list (GNOME Shell's own slider scales every backlight
  together); a second internal panel is not set separately.

## Slice 2: Geluid, Eigenschappen, Alle geluidsapparaten, Volumemixer

### What was built

Host (`host/src/services/sound.rs`, `sound/{pulse,client,model,alerts,canberra,tests}.rs`,
`services/system/{audio,audio_monitor}.rs`; `api-methods/system.json`; `host/i18n/sound.tsv`):

* `sound/pulse.rs`: libpulse 17 and libpulse-mainloop-glib through `dlib!` (amendment 39): the context, the
  introspection of sinks, sources, sink inputs, source outputs, clients and cards, every write the pages need,
  ext-stream-restore, peak-detect record streams and libpulse's own pure volume functions (pa_cvolume_scale,
  _set_balance, _set_fade, _set_position, pa_channel_map_can_balance ...). The struct layouts are those of
  /usr/include/pulse on x86_64; a C program printed every size and the offset of every field read, and
  `sound/tests.rs` asserts them all (`offset_of!`).
* `sound/client.rs`: one PulseAudio client per host process, on a thread of its own with its own GLib MainContext and
  libpulse's GLib main loop on it (amendment 39). On READY it subscribes (sinks, sources, streams, clients, cards,
  server), reads everything plus ext-stream-restore's event role, and keeps the model from the subscription events;
  a burst of changes is published 40 ms after its last event (one `Event::Changed`). A lost server is retried after
  1, 2, 5, then every 10 s; state callbacks are deferred to the loop (libpulse calls them from inside
  pa_context_connect). Callers hand the thread jobs and wait 5 s. Level meters are GNOME Settings' level bars (float32,
  mono, 25 Hz, PEAK_DETECT | DONT_MOVE | ADJUST_LATENCY) with GNOME Settings' application.id
  `org.gnome.VolumeControl`, so GNOME Shell's microphone indicator and every libgvc mixer skip them as they skip
  GNOME's own (gnome-shell's volume.js filters that id); a meter has a 30 s lease that the page renews every 10 s and
  stops on the page's cleanup. A test build connects only to WIN11_TEST_PULSE_SERVER (never set), so no test can
  reach a real server.
* `sound/model.rs`: the model as plain data, `sound.state`'s JSON and GNOME Settings' device lists (libgvc's UI
  devices: every available port of every card, a port whose profile is not active carries the profile to switch to,
  keeping the other direction as gvc_mixer_ui_device_get_best_profile does; then every sink or source without a card;
  monitors are never inputs). The streams list leaves out the event-role streams (Systeemgeluiden is one slider),
  GNOME's, pavucontrol's and WinPoint's meters and recordings of a monitor, as GNOME's Volume Levels does.
* Methods (§5.5): `sound.state`, `sound.setDefault` (with GNOME's profile switch for a port of another profile,
  then the port, then the default), `sound.setVolume` (pa_cvolume_scale on the loudest channel, as gvc; above 100 %
  only up to PA_VOLUME_UI_MAX, +11 dB = about 153 %, while allow-volume-above-100-percent is on; a volume above 0
  unmutes), `sound.setMute`, `sound.setBalance` (balance, fade, LFE; `left`/`right` for Windows' two channel sliders
  as one write), `sound.setProfile` ("off" = Niet toestaan, the profile before is remembered as the card's
  `restore`), `sound.setPort`, `sound.moveStream`, `sound.setEventVolume` (gvc_mixer_event_role_push_volume:
  ext-stream-restore's `sink-input-by-media-role:event`, device and mute kept; without an entry GNOME's 100 % shows),
  `sound.test` (libcanberra: GNOME's audio-channel-<position> forced onto that channel of the sink, media.role test,
  with GNOME's fallbacks; or an alert file), `sound.meter`, `sound.alerts.get`, `sound.alerts.set`. Every dangerous
  method checks its arguments against the server's state in a dry run and writes nothing.
* `sound/alerts.rs`: GNOME Settings 50's alert chooser (cc-alert-chooser-page.c): Geen geluiden = event-sounds
  false; Standaard = event-sounds true and theme-name to its default (Yaru on Ubuntu); Klik / Snaar / Schommel / Brom
  = GNOME's `$XDG_DATA_HOME/sounds/__custom` theme (index.theme with Name, Inherits = the default theme,
  Directories = ., kept keys; the bell-terminal.ogg and bell-window-system.ogg links to
  /usr/share/sounds/gnome/default/alerts/<name>.ogg; the directory's mtime renewed), event-sounds true and
  theme-name "__custom". Opstartgeluid afspelen = `startup-sound` "warty-startup" / "" (GNOME Settings' mapping on
  Ubuntu, read from its binary's strings next to startup_sound_row). Every key is checked (write_values dry run)
  before a file is written; a dry run writes neither.
* system.audio.* (amendment 39, one source of truth): `system/audio.rs` asks the Sound client first
  (`sound::audio_state`, `audio_apply`, `audio_set_sink`) and keeps wpctl as the fallback (no libpulse, or no pulse
  server). The payload is unchanged; a sink's `id` is PipeWire's object id from the pulse proplist (what wpctl gave,
  so Bluetooth's follow-the-headset code in `system/bluetooth.rs`, which inspects it with wpctl, keeps working).
  `system/audio_monitor.rs` listens to the client's changes, and runs pw-dump only when the client does not connect
  within its first 1.5 s (libpulse loads but no pulse server answers: PipeWire without pipewire-pulse; review fix
  D-2).
  No literal of either file was removed (drift rule 1).

Native (`pages/sound.rs`, `pages/sound_device.rs`, `pages/volume_mixer.rs`):

* Geluid (GNOME path when sound.state is available, else the old system.audio page, kept): Uitvoer (Kies waar je
  geluid wilt afspelen: GNOME's output entries as radio rows, glyph by port or form factor, title = the port (Luidsprekers) and the
  adapter below it, a chevron to Eigenschappen; Een nieuw uitvoerapparaat koppelen · Apparaat toevoegen →
  `go_with("bluetooth", {add: 1})`; Volume with mute), Invoer (the same for inputs with Een nieuw invoerapparaat
  koppelen (amendment 2); Volume with the live level meter of the default input), Geavanceerd (Alle
  geluidsapparaten, Volumemixer, Meer geluidsinstellingen). Picking an entry calls setDefault (with the profile switch
  when its port needs one). Volume sliders write every 60 ms while dragged and once on release and ignore the
  server's echo for 0.7 s; the maximum follows allow-volume-above-100-percent (the page rebuilds when it changes).
* Meer geluidsinstellingen (amendment 3): the classic Geluid dialog with its Geluiden tab: Geluidsschema (Geen
  geluiden, Standaard, Klik, Snaar, Schommel, Brom; only installed alert files) with Testen (the choice plays at once,
  as GNOME's chooser does), and Opstartgeluid afspelen when Ubuntu's schema is installed.
* Eigenschappen (data page, params {kind, id, card?}; title = the entry's name): Audio (Toestaan / Niet toestaan =
  the card's off profile and back to the remembered one; after Niet toestaan the page shows the card with Toestaan),
  Instellen als standaardgeluidsapparaat, Uitvoerinstellingen / Invoerinstellingen: Indeling (the card's available
  profiles, GNOME's Configuration) with Testen (GNOME's speaker test: one button per channel of the sink), Poort
  (more than one available port), Volume / Invoervolume, Linkerkanaal and Rechterkanaal (GNOME's balance), Balans
  voor en achter and Subwoofer (GNOME's fade and LFE, only when the channel map has them), Volume boven 100%
  toestaan (gs toggle on allow-volume-above-100-percent); for an input Je microfoon testen · Test starten / Test
  stoppen with the level meter.
* Alle geluidsapparaten (amendment 1): Uitvoerapparaten and Invoerapparaten, every entry plus the cards set to Niet
  toestaan ("Uitgeschakeld"), each opening Eigenschappen.
* Volumemixer: Systeem (Uitvoerapparaat and Invoerapparaat drop-downs of GNOME's entries, Volume), Apps
  (Systeemgeluiden = the event role, then one expander per app stream with its volume and mute in the header and its
  Uitvoerapparaat / Invoerapparaat inside, which moves the stream). A new or gone stream rebuilds the list, a volume
  change only moves the sliders.

### Plan controls changed or dropped (with the reason)

* sound "Kies waar geluid moet worden afgespeeld": kept WinPoint's existing "Kies waar je geluid wilt afspelen"
  (amendment 28: existing strings stay until a verified Windows string replaces them; windows.json's is from memory).
* sound "Volume + dempen": sound.setVolume / setMute instead of system.audio.set (system.audio.* itself now runs on
  the same client, so both paths agree).
* sound "Opstartgeluid afspelen": in Meer geluidsinstellingen › Geluiden (amendment 3).
* sound related link "Geluiden bij meldingen": dropped from the GNOME page (Windows' Sound page has none; it stays on
  the page without a pulse server).
* Windows-only rows left out (windows.json says omit): Mono-audio, Veelvoorkomende geluidsproblemen oplossen, Naam
  wijzigen, Audioverbeteringen, Ruimtelijk geluid, the Afspelen / Opnemen / Communicatie tabs, and the Volumemixer's
  "Geluidsapparaten en volumes voor alle apps opnieuw instellen" (GNOME has no such function; it would reset
  WirePlumber's state for the whole session).
* sound-device "Audio: Toestaan / Niet toestaan": a button as Windows has it (the plan said a drop-down).
* sound-device "Linkerkanaal / Rechterkanaal": Windows' two sliders map onto GNOME's balance (one write of volume and
  balance through setBalance {left, right}); GNOME's Fade and Subwoofer sliders are WinPoint additions under them,
  shown only for a surround channel map.
* sound-device "Poort": a WinPoint row (GNOME's port chooser), only with more than one available port.
* sound-device "Je microfoon testen": the live level, no Windows-style percentage result.
* sound "Meer geluidsinstellingen": GNOME's Standaard resets theme-name (g_settings_reset); `write_values` cannot
  reset a Service row, so it writes the schema's default value instead (same theme; a later change of the default
  would not follow). Patch for I below.
* `input-feedback-sounds` (allow/system.rs, Write since B0): no control; GNOME Settings 50 has none either.

### Notes for I

1. `sound.changed` carries the whole `sound.state` (the contract wrote `{}`); `sound.level` is `{kind, name, peak}`
   (the contract wrote `{name, peak}`). Optional arguments beyond §5.5, all in system.json's docs:
   `sound.setDefault {card?, profile?}`, `sound.setBalance {volume?, left?, right?}`, `sound.setEventVolume
   {mute?}`, `sound.test {alert?}`, `sound.alerts.set {alert?}`.
2. system.rs (frozen) still publishes `system.audio.changed` for the taskbar and Snelle instellingen; it is now fed by
   the same pulse subscription as `sound.changed`, so the two never disagree.
3. GNOME's Standaard alert exactly (g_settings_reset): a reset for a method's own rows in
   `host/src/services/gsettings.rs`, next to `write_values`:

   ```rust
   /// `gsettings.reset` for host method `method`'s own rows (Service(method), Write or Mirrored), default backend.
   pub(crate) fn reset_values(method: &str, schema: &str, path: Option<&str>, keys: &[&str], lang: Lang, dry_run: bool) -> Result<Value, ApiError> {
       let mut env = Env::new(&Config::default(), None);
       let keys: Vec<String> = keys.iter().map(|k| (*k).to_owned()).collect();
       reset_as(&mut env, schema, path, &keys, lang, dry_run, Caller::Service(method))
   }
   ```

   with `reset` becoming `reset_as(.., Caller::Api)` (its two `refuse(.., Caller::Api, ..)` calls take the caller);
   then `alerts.rs` `Keys` gains `reset` and Standaard resets theme-name.
4. Registry (optional keywords): sound + "invoer"/"input", "microfoon"/"microphone", "invoerapparaat"/"input device";
   sound-device + "linkerkanaal"/"left channel", "rechterkanaal"/"right channel", "indeling"/"format", "testen"/"test";
   volume-mixer + "systeemgeluiden"/"system sounds", "app-volume"/"app volume"; sound + "geluidsschema"/"sound scheme",
   "opstartgeluid"/"startup sound".
5. VALIDATION_SETTINGS.md (amendment 62's second half, I's): settings-system.py runs a private PipeWire (pipewire,
   wireplumber with every hardware monitor disabled, pipewire-pulse, null devices from pw-cli) and checks the pulse
   state, not what a real card does; card profiles, ports and Niet toestaan are covered by the host unit tests on
   fixtures only (a null sink has no card).

### Allow rows, i18n, tests

* Allow rows: none added or changed. B0's rows in `allow/system.rs` cover every key: allow-volume-above-100-percent
  (Write; the Eigenschappen toggle and the slider maximum), event-sounds and theme-name (`Service("sound.alerts.set")`),
  org.gnome.shell.ubuntu startup-sound (Write, optional ubuntu). A host test reads all four with their modes and
  checks that another method may not write sound.alerts.set's rows.
* Host texts: eight `sound.*` rows in `host/i18n/sound.tsv` (`#@ new`, listed in `BASELINE-CHANGES.rust-settings-d`);
  the stream names libpulse and libcanberra put on the server ("Peak detect", "Speaker test", "Alert sound") are in
  `allow-host-settings-d.txt` (nobody sees them in WinPoint). system/audio.rs and audio_monitor.rs only gained
  literals (drift rule 1). UI strings: 90 rows in `windows-glossary/d.tsv` (Windows' own from windows.json verified
  no, Eigenschappen from glossary.tsv yes, two renames, the rest WinPoint's). The page without a pulse server keeps
  "Geluiden bij meldingen" inside `sound()`, where glossary-context.tsv's `sound.rs:sound/*` row expects it.
* Host unit tests (`--lib`): 595 passed, 1 failed, 19 ignored (615; this slice adds 11 in `services::sound::tests`:
  the libpulse layouts, volume math, balance / fade / LFE through libpulse, GNOME's device lists, the stream filter
  and the state, system.audio's payload, the test-build guard, the alert chooser value and plan, the custom theme and
  keys on a memory backend, the dry run and refusal, the allow rows). The failure is
  `services::update::elf::tests::our_binaries_equal_readelf` (open issues).
* Native unit tests (`cargo test --release`): 76 passed, 1 ignored; the settings crate 41 (this slice: 3 in
  `pages::sound::tests`, 4 in `pages::sound_device::tests`, 1 in `pages::volume_mixer::tests`).
* Harness (`settings-system.py`, amendment 62): nl 62 checks, en 68, none failed. The scenario starts pipewire,
  wireplumber (a `wireplumber.conf.d` fragment in the run's XDG_CONFIG_HOME disables the alsa, bluez, v4l2 and
  libcamera monitors; the sandbox has no /dev/snd anyway) and pipewire-pulse, makes two null sinks and a null source
  with `pw-cli create-node adapter` (object.linger), and plays /dev/zero with pw-play for the Volumemixer. It checks
  the GNOME path of Geluid (both lists, every named control, no wpctl page), choosing an output, the volume slider and
  the mute button on the sink, system.audio.get on the same client with PipeWire's object ids, Meer
  geluidsinstellingen (Opstartgeluid afspelen writes startup-sound), sound.alerts.set click and default (the
  __custom theme and its link, then Yaru), Eigenschappen (a data page; Linkerkanaal / Rechterkanaal reach the sink as
  left 100, right 50, balance -0.5), Alle geluidsapparaten, the Volumemixer rows, moveStream and the event role through
  ext-stream-restore. The first run found a real FFI bug: `pa_ext_stream_restore_write` takes an array of the structs,
  not of pointers (the host died with SIGSEGV on Systeemgeluiden); fixed in 984c32e9, and every other prototype was
  compared with the headers again. It also showed a lone microphone row drawn as a check box (fixed in a5a1a410).
* `checks.sh d`: 0 errors; warnings at the base (check_rust 57, drift 15, native-dutch-only 48); the installer check
  refuses with rc=70 ("XDG_RUNTIME_DIR is not 0700") as before this slice.

### Open issues

* `services::update::elf::tests::our_binaries_equal_readelf` fails: the release host binaries now need libm.so.6 (the
  only libm symbol they import is `pow`, and the only `pow` in host/src is B0's settings_mirror.rs). Batches c and e
  fail the same test and every batch worktree's win11-app has the entry, so it is not this slice's; the recording
  `tests/update/fixtures/compat/readelf-needed.txt` (tests/update/test_compat.sh --record-readelf) is I's to renew.
* Card profiles, ports, Audio Niet toestaan / Toestaan and the HDMI profile switch could not run in the harness (a
  null sink has no card); they are covered by the host fixtures and the native unit tests only.
* sound.test (libcanberra) and the level meter's peaks were not checked in the harness beyond the controls being
  there (a null source has no signal).
* With no configured default source, pipewire-pulse reports the default sink's monitor; the input list then selects
  nothing, as GNOME's does, and Volume under Invoer meters that monitor.

## Slice 3: Energie en batterij, Opslag, Opslaginzicht, Multitasking, Schermtijd

### What was built

Host (`host/src/services/battery.rs`, `wellbeing.rs`, `storage.rs`, `storage/sense.rs`, `storage/test_bus.rs` (tests
only); `api-methods/system.json`; `host/i18n/{battery,storage}.tsv`):

* `battery.charging.get {}` → `{available, supported, enabled, start, end, devices}`: UPower's batteries that power
  the computer (EnumerateDevices, Type 2, PowerSupply; a mouse battery or the adapter never count), `supported` when one
  reports ChargeThresholdSupported (GNOME shows Battery Charging only then), `enabled` when every supported one has
  ChargeThresholdEnabled, `start`/`end` its ChargeStart/EndThreshold.
* `battery.charging.set! {enabled}` → the same state read back after `EnableChargeThreshold(b)` on every supported
  battery through `call_interactive` (polkit org.freedesktop.UPower.enable-charging-limit, allow_active yes: no
  dialog at the seat); `unsupported battery.unavailable` / `battery.noChargeLimit`; a dry run checks and sends
  nothing. Event `battery.changed` with the state after a set and whenever UPower reports a ChargeThreshold property
  change (a watcher on the Battery service's own GLib context, once per burst: a one-shot 200 ms timer that every
  signal of the burst restarts; no periodic tick, review fix D-3).
* `battery.history {type?, timespan?, resolution?}` → `{available, type, timespan, device, points: [[time, value,
  state]]}`: UPower's GetHistory (`suu` → `a(udu)`) of the first battery with HasHistory, oldest first, empty and
  repeated samples dropped; `available: false` without one.
* `wellbeing.history {}` → `{available, today, active, parental, days: [{date, weekday, seconds}]}`: gnome-shell 50's
  own record, `$XDG_DATA_HOME/gnome-shell/session-active-history.json` (js/misc/timeLimitsManager.js, read out of
  libshell-18.so: an array of `{oldState, newState, wallTimeSecs}`, UserState 0/1), validated as gnome-shell's
  `_loadTransitions` does (a malformed file is `available: false`, GNOME's "Screen Time Unavailable"), its 14-week
  window and "always keep the last entry" rule, active periods summed per local day (split at midnight through
  glib's time zone, so a 25-hour day counts 25 hours), every day from the first record to today, an open period up
  to now. `parental`: AccountsService FindUserById + com.endlessm.ParentalControls.SessionLimits LimitType ≠ 0
  (gnome-shell's `sessionLimitsEnabled`), 500 ms timeouts.
* `storage.cleanTemp! {sense?}` (`storage/sense.rs`): without `sense` GNOME Settings' own Delete Temporary Files…
  call, gsd-housekeeping's `RemoveTempFiles` on the session bus (org.gnome.SettingsDaemon.Housekeeping, read from
  /usr/libexec/gsd-housekeeping and gnome-control-center's strings); without gsd-housekeeping (Hyprland) the host
  does the same. `sense: true` is Opslaginzicht nu uitvoeren: gsd-housekeeping's hourly `ldsm_purge_trash_and_temp`
  with org.gnome.desktop.privacy now (temporary files older than old-files-age days when remove-old-temp-files,
  Prullenbak items deleted more than old-files-age days ago when remove-old-trash-files; `temp`/`trash` null when
  off). The purge follows gsd's `delete_recursively_by_age`: only this user's entries by ctime, children first, a
  directory only once empty (and by its ctime after its children went), never across a mount, never through a
  symbolic link: the walk holds each directory open and goes on with openat/fstatat/unlinkat (O_NOFOLLOW, the inode
  checked again), so a directory swapped for a link meanwhile cannot redirect it. The Prullenbak part reads each
  `info/*.trashinfo` DeletionDate (local time) and removes the item with its info (an item without info is left). A
  test build touches only `WIN11_TEST_TMPDIR` and no Prullenbak. Optional `sense` documented in system.json.
* `storage/test_bus.rs` (cfg(test)): a generic in-process D-Bus fake on the sandbox's private session bus (objects
  with their real introspection XML, reached by unique name, calls recorded with the interactive flag), used by the
  battery, wellbeing and storage tests.

Native (`pages/{power,storage,storagesense,multitasking,screentime}.rs`, `native/shell/src/flyouts/menus/taskview.rs`):

* Energie en batterij (amendment 20): the battery hero; Energie: the expander Scherm en slaapstand (the existing four
  rows, unchanged strings and system.power.settings.* logic, plus GNOME's notice "Als je automatisch slapen
  uitschakelt, verbruikt de pc meer energie…" while automatic sleep of the power source that counts is Nooit), the
  expander Besturingselementen voor deksel en aan/uit-knop with one drop-down "Als ik op de aan/uit-knop druk, gaat
  mijn pc naar" (power-button-action: Niets doen / Slaapstand / Sluimerstand (caps.hibernate, or when already set) /
  Afsluiten = GNOME's Power Off, `interactive`), Energiestand; Batterij (only with a battery, built once and shown
  while one is present): Energiebesparing (Altijd energiebesparing gebruiken = the power-saver profile through
  system.power.settings.set, off = balanced; Energiebesparing automatisch inschakelen bij een laag batterijniveau =
  power-saver-profile-on-low-battery; the expander only with power-profiles-daemon's power-saver profile, as GNOME),
  Batterijpercentage (show-battery-percentage, Mirrored to taskbar.batteryPercent), Batterij opladen (Maximale
  lading / Batterijgezondheid behouden with GNOME's subtitles and the end threshold; only when supported; follows
  battery.changed), Batterijniveaus (Afgelopen 24 uur / Afgelopen 7 dagen inside the expander: battery.history
  resampled onto a 15-minute or hourly grid, the kit's LineChart, refreshed every 5 minutes), then the health, cycles
  and power use. A pc without a battery gets Energiebesparing in Energie (Windows 24H2) and the existing Netstroom
  card. Aan/uit unchanged.
* Opslag: Windows' Opslagbeheer group: Opslaginzicht (a plain card with the switch and a chevron button to its page,
  `link-storagesense`, see Notes for I 4; the switch writes remove-old-trash-files and remove-old-temp-files in one
  gsettings.set and is on while either is, amendment 21),
  then the existing Ruimte vrijmaken. Categories and drives unchanged.
* Opslaginzicht: Tijdelijke bestanden opschonen (remove-old-temp-files); Automatisch opschonen van gebruikersinhoud:
  Opslaginzicht (the same master switch); Opschoonschema's configureren: "Bestanden in mijn Prullenbak verwijderen als
  ze er langer in staan dan:" (Nooit = remove-old-trash-files false, else true with old-files-age; Windows' 1, 14, 30,
  60 dagen, GNOME's 1 uur = 0, another current age as its own option; "Dezelfde periode geldt voor tijdelijke
  bestanden."); Opslaginzicht nu uitvoeren (storage.cleanTemp {sense: true}, a toast with what was freed; off while
  both purges are off); GNOME's Prullenbak nu leegmaken (files.emptyTrash) and Tijdelijke bestanden nu verwijderen
  (storage.cleanTemp), each after a question.
* Multitasking (amendments 11, 12): Vensters uitlijnen (edge-tiling); Bureaubladen: "Op de taakbalk alle geopende
  vensters weergeven" (taskbar.allDesktops) and "Alle geopende vensters weergeven wanneer ik op Alt+Tab druk"
  (taskView.allDesktops plus GNOME's app-switcher and window-switcher current-workspace-only = not all, one
  gsettings.set with `writes`, a locked key left out; the row shows GNOME's app-switcher key, follows its outside
  changes and aligns taskView.allDesktops to it, review fix D-7), both "Alleen op het bureaublad dat ik gebruik / Op
  alle bureaubladen"; then GNOME's Lege bureaubladen automatisch verwijderen / Vast aantal bureaubladen (dynamic-workspaces)
  with Aantal bureaubladen (num-workspaces 1-36, shown for a fixed number) and Bureaubladen alleen op het
  hoofdbeeldscherm / op alle beeldschermen (workspaces-only-on-primary); last Actieve hoek (enable-hot-corners) with
  "Geldt in de GNOME- en Ubuntu-sessie".
* Taakweergave (taskview.rs, amendment 12's consumer): with taskView.allDesktops off (the default) Alt+Tab shows the
  windows of the desktop in use and those on every desktop. The desktop in use is windows.list's `activeWorkspace`
  when the extension reports it (Notes for I 1), else the focused window's workspace, else the most recently used
  window's; with nothing to go by every window shows.
* Schermtijd (GNOME's Wellbeing): Schermtijd: GNOME's statistics row as a card (the selected day with its time and
  "Gemiddelde <weekdag>", the week with its total and "Gemiddelde week", a bar per day from time.firstDayOfWeek
  (Standaard = Monday), a click selects a day, the daily limit as a dashed line, Vorige week / Volgende week),
  "Schermtijd is niet beschikbaar" before a record, "Vastleggen van schermtijd is uitgeschakeld" with Inschakelen
  while history-enabled is off, Schermtijd vastleggen (history-enabled, GNOME's text about the data staying on the
  device); read again every minute. Schermlimieten: Ouderlijk toezicht's banner when wellbeing.history says
  `parental` (with "Ouderlijk toezicht openen" = apps.launch org.freedesktop.MalcontentControl.desktop when
  caps.malcontent), Limiet voor schermtijd (daily-limit-enabled) with Dagelijkse limiet (hours and minutes,
  daily-limit-seconds, at least a minute, written 0.5 s after the last change) and Grijswaarden (grayscale), both
  insensitive while the limit is off or Parental Controls manage it. Pauzeherinneringen: Herinneringen voor je ogen
  and Herinneringen om te bewegen (eyesight / movement in selected-breaks, the other entries kept), Schema voor
  bewegingspauzes (GNOME 50's four schedules, read from gnome-control-center's table: 1 min every 20, 2 every 20, 3
  every 30, 5 every 30 minutes, plus another current one), Geluiden (play-sound of both breaks in one write,
  insensitive while no break is on).

### Plan controls changed or dropped (with the reason)

* power "Energiestand, Time-outs …, Aan/uit-acties (ongewijzigd)": unchanged logic and strings, regrouped into
  Windows' expanders (amendment 20). The expander keeps WinPoint's "Scherm en slaapstand" (Windows 23H2's name;
  24H2's "Time-outs voor scherm, slaapstand en sluimerstand" is unverified in windows.json, amendment 28).
* power "Batterijpercentage weergeven in het systeemvak": Windows' own toggle "Batterijpercentage" with its subtitle
  (windows.json, the 2025 addition).
* power "Altijd energiebesparing gebruiken": turned off it sets balanced (GNOME's default; performance when a machine
  has no balanced). The Energiebesparing expander shows only with a power-saver profile (GNOME's Automatic Power
  Saver also needs power-profiles-daemon).
* power "Als ik op de aan/uit-knop druk": one column (GNOME has one value); Windows' "Het beeldscherm uitschakelen"
  has no GNOME value and is not offered; the lid rows are left out (amendment 20; GNOME Settings has none).
* power "Batterijniveaus": a line of the charge level over 24 hours or 7 days. Windows' per-app "Batterijgebruik" is
  left out (no GNOME function, windows.json: omit).
* storage "Opslagzin": named Opslaginzicht (glossary.tsv, §2), in Windows' Opslagbeheer group; the switch is named
  `st-storage-storage-sense` (`st-storagesense-storage-sense` on its page): it binds two keys, so no single key name.
* storagesense "Tijdelijke bestanden opschonen": a switch row as the plan says (Windows has a check box). Windows'
  "Opslaginzicht uitvoeren" cadence and the Downloads row are left out (gsd-housekeeping runs hourly; windows.json:
  omit). "Opslaginzicht nu uitvoeren" added (amendment 21) and insensitive while both purges are off.
* storagesense "Prullenbak nu leegmaken": files.emptyTrash (the plan), not gsd's EmptyTrash, so Verkenner's Prullenbak
  view and the trash size cache follow; "Tijdelijke bestanden nu verwijderen" is gsd's RemoveTempFiles exactly.
* multitasking: Windows' snap sub-options (WinPoint's snap layouts have no settings; tiling-assistant keys are never
  written), "Venster schudden via titelbalk" (Windows only) and the Ubuntu Dock's monitor isolation (dash-to-dock is
  forbidden) are left out; GNOME's "Reopen Windows After Login" is batch E's (signin).
* screentime "Limiet voor schermtijd + Dagelijkse limiet": a switch row with Dagelijkse limiet and Grijswaarden on
  the rows below it (not an expander with the switch in its header: Notes for I 4); the limit as two number boxes
  (uur, min), as GNOME's duration row. Herinneringen om te bewegen likewise: its switch row, then the schedule row
  (insensitive while movement is not selected). "Grijswaarden": a switch (plan: check box), as GNOME's
  switch row.
* screentime "Een geluid afspelen wanneer een pauze eindigt": titled Geluiden with that subtitle (GNOME's Sounds row);
  it writes play-sound of both break schemas, its state is the movement break's (they are written together).
* The `st-…` names of controls bound to settings or several keys: `st-multitasking-taskbar-all-desktops`,
  `st-multitasking-app-switcher-current-workspace-only` (the Alt+Tab drop-down, B0's gs.rs test names it),
  `st-screentime-selected-breaks-eyesight` / `-movement`, `st-screentime-interval-seconds` (the schedule),
  `st-screentime-movement-play-sound` (Geluiden), `st-screentime-daily-limit-seconds` (with `-hours` / `-minutes`),
  `st-storagesense-old-files-age` (the Prullenbak row), `st-power-charging-maximize` / `-preserve`, `st-power-saver`.

### Notes for I

1. `extension/panelBridge.js` (multi-monitor worktree): the desktop in use for Taakweergave (and for the taskbar's
   `taskbar.allDesktops`, section 8.3). In `windows.list` and in the `windows.changed` event, beside `windows`:

   ```js
   // _queueWindows(), instead of the fingerprint of the windows alone:
   const windows = this._userWindows().map(window => this._windowJSON(window));
   const activeWorkspace = global.workspace_manager.get_active_workspace_index();
   const fingerprint = JSON.stringify([windows, activeWorkspace]);
   if (fingerprint !== this._lastWindows) {
       this._lastWindows = fingerprint;
       this._emit('windows.changed', {windows, activeWorkspace});
   }
   // _call:
   if (method === 'windows.list')
       return {supported: true, windows: this._userWindows().map(window => this._windowJSON(window)),
           activeWorkspace: global.workspace_manager.get_active_workspace_index()};
   ```

   taskview.rs reads `activeWorkspace` already; until then it takes the focused or most recently used window's
   workspace (a switch to an empty desktop then still shows the previous desktop's windows).
2. Events beyond §5.5: `battery.changed` carries the charging state (the contract wrote `{}`). Optional arguments:
   `storage.cleanTemp {sense?}`, `battery.history {resolution?}` (system.json's docs).
3. Registry (optional keywords): power + "aan/uit-knop"/"power button", "batterij opladen"/"battery charging",
   "batterijpercentage"/"battery percentage", "batterijniveaus"/"battery levels"; storage + "opslaginzicht"/"storage
   sense"; storagesense + "prullenbak leegmaken"/"empty recycle bin", "tijdelijke bestanden verwijderen"/"delete
   temporary files", "opslaginzicht nu uitvoeren"/"run storage sense now"; multitasking + "aantal
   bureaubladen"/"number of desktops", "hoofdbeeldscherm"/"main display", "taakweergave"/"task view"; screentime +
   "dagelijkse limiet"/"daily limit", "grijswaarden"/"greyscale", "ouderlijk toezicht"/"parental controls",
   "bewegingspauzes"/"movement breaks".
4. kit (`native/kit/src/controls/cards.rs`, frozen): a control inside a clickable card or an expander header (both a
   `gtk::Button`) never takes a real pointer click. GtkButton's click gesture runs in the capture phase and claims the
   sequence on release, before the switch's, check box's or inner button's own gesture can: the button is "clicked"
   (the card navigates, the expander opens) and the control stays as it was. The harness shows it
   (settings-system.py `kit_header_control`, a known 'kit' check): a real click on Meldingen' toast switch in its
   expander header leaves notifications.showToasts unchanged; the same real clicks on switches of plain cards work.
   (A slider still drags: GtkRange claims on press.) Batch D's pages no longer do it (this slice: Opslaginzicht's row
   is a plain card with the switch and its own chevron button, Limiet voor schermtijd, Herinneringen om te bewegen and
   Batterijniveaus' range have their controls on rows of their own; slices 1 and 2 changed the same way: Beeldscherm's
   Schaal, Helderheid with a light sensor, Nachtlampje's row and the Hyprland night light, Nachtlampje plannen, the
   Volumemixer's app rows). Other pages still do: about.rs (the two Kopiëren buttons), background.rs:606 and
   colors.rs:159 (drop-downs), network.rs:607 and :724 (buttons), notifications.rs:51 (the switch), and every
   `link_card_with(..).set_control(..)`. Suggested kit fix: in `SettingsCard::build` for a clickable card, put the
   control box on a `gtk::Overlay` above the button (halign End, a margin for the chevron) instead of inside it, with a
   placeholder in the button's row joined to the control box by a horizontal `gtk::SizeGroup` so the text keeps its
   room; `widget()` becomes the overlay, `button()` the stored button (CSS keeps `button.w11-settings-card`).

### Allow rows, i18n, tests

* Allow rows: none added or changed; B0's rows in `allow/system.rs` cover every key these pages bind. A host test
  (`services::wellbeing::tests::the_keys_of_these_pages_have_their_allow_rows`) reads each one on a memory backend
  with its mode: the screen-time-limits and break-reminders keys, the privacy purge keys, power-button-action and
  power-saver-profile-on-low-battery Write, idle-dim and sleep-inactive-* Service(system.power.settings.set),
  show-battery-percentage Mirrored(taskbar.batteryPercent), enable-hot-corners, mutter's edge-tiling,
  dynamic-workspaces and workspaces-only-on-primary, num-workspaces and both switchers' current-workspace-only Write.
* Host texts: `battery.unavailable`, `battery.noChargeLimit` (`host/i18n/battery.tsv`), `storage.tempUnavailable`,
  `storage.tempFailed` (`host/i18n/storage.tsv`), all `#@ new` and in `BASELINE-CHANGES.rust-settings-d`;
  `wellbeing.tsv` needs no row (wellbeing.history has no error of its own). UI strings: 108 rows in
  `windows-glossary/d.tsv` (Windows' own from windows.json verified no, Prullenbak leegmaken and Slaapstand from
  glossary.tsv yes, the rest WinPoint's wording of GNOME's texts).
* Host unit tests (`--lib`): 611 passed, 1 failed, 19 ignored (631; this slice adds 16: `services::battery::tests` 5,
  `services::wellbeing::tests` 5, `services::storage::sense::tests` 6). The failure is the known
  `services::update::elf::tests::our_binaries_equal_readelf` (libm, slice 2's open issue). The D-Bus side runs
  against `storage/test_bus.rs` on the sandbox's private session bus: UPower's EnumerateDevices, the batteries'
  properties, EnableChargeThreshold (recorded with the interactive flag set, nothing sent in a dry run) and
  GetHistory; AccountsService's FindUserById and malcontent's SessionLimits; gsd-housekeeping's RemoveTempFiles (not
  EmptyTrash, nothing in a dry run). The purges run on temporary directories with a moved clock: another user's
  files, a symbolic link out of the tree, a directory emptied just now, Prullenbak items older and newer than the
  age, an item without its info and an info without its item. No `tests/dbus-fakes/fakes/d/` fake was needed.
* Native unit tests (`cargo test --release`): 87 passed, 1 ignored; the settings crate 51 (this slice: 4 in
  `pages::power::tests`, 2 in `pages::storagesense::tests`, 1 in `pages::multitasking::tests`, 3 in
  `pages::screentime::tests`), the shell 14 (1 in `flyouts::menus::taskview::tests`).
* Harness (`settings-system.py`, a UPower stand-in from `tests/gnome/lib/standins/d.py` on the private system bus):
  nl 118 checks, en 129, each with one failure, the known 'kit' check above (Notes for I 4); every slice-3 check passes
  (nl 49, en 54 with the no-Dutch ones): the power-button drop-down writes power-button-action, Batterijpercentage
  writes show-battery-percentage, Batterijgezondheid behouden sends EnableChargeThreshold(true) with interactive
  authorization, battery.history reads GetHistory oldest first and the chart is drawn; the Opslaginzicht switch writes
  both purge keys and follows an outside write; Tijdelijke bestanden opschonen, the Prullenbak drop-down's 14 dagen
  (both keys), an outside age of 7 days as its own option, storage.cleanTemp {sense: true}; edge-tiling, Alt+Tab's
  Op alle bureaubladen (taskView.allDesktops and both GNOME switchers false), Vast aantal bureaubladen with the number
  row, the hot corner; wellbeing.history on gnome-shell 50's own record, the chart or GNOME's Screen Time
  Unavailable, recording off, both break reminders, the movement schedule (180 s every 1800 s), the daily limit. The
  first runs found two bugs, fixed in a20adb29: the charging radios' handler lived only through weak references
  (EnableChargeThreshold was never sent), and every control inside a clickable header ignored the pointer.
* `checks.sh d`: 0 errors; warnings at the base (check_rust 57, drift 15, native-dutch-only 48); the installer check
  refuses with rc=70 ("XDG_RUNTIME_DIR is not 0700") as before. Release builds: host 31 warning lines, native 20 (the
  base counts), none in this slice's files.

### Open issues

* Taakweergave knows the desktop in use exactly only once panelBridge.js reports `activeWorkspace` (Notes for I 1);
  until then a switch to an empty desktop still shows the previous desktop's windows with taskView.allDesktops off.
* Energiebesparing needs power-profiles-daemon's power-saver profile; the harness has no power-profiles-daemon (a
  stand-in cannot carry its `aa{sv}` Profiles through the JSON spec), so those two rows and Altijd energiebesparing
  gebruiken are covered by unit tests only. (Resolved in the harness pass below: `standins/d.py` `start_text`.)
* Parental Controls' banner (malcontent) and "Ouderlijk toezicht openen" are covered by the host fake only;
  malcontent-control is not installed here.
* gnome-shell in the harness had not written session-active-history.json yet when the page opened, so the harness saw
  "Schermtijd is niet beschikbaar"; the chart itself is covered by the native unit tests (weeks, averages) and the host
  tests (the file, the days).
* "Tijdelijke bestanden nu verwijderen" deletes every file of this user in the temporary folder, as GNOME does (apps
  that still use one may notice); the harness does not press it (its temporary folder is the run's own).
* Other batches' pages still put controls inside clickable headers (Notes for I 4).

## Harness pass: settings-system.py over all of batch D

The scenario (`tests/gnome/scenarios/settings-system.py`) now follows plan.json "testing" (GNOME HARNESS) for every
page of §4.5, and runs under `--strace` in both languages (Energie's Aan/uit can reach `systemctl`).

### What every page gets

* Opened by its id (`page()`: `h.open('settings:<id>')`), the window maximised (Windows' Settings usually is; Mutter's
  `maximize`), a screenshot `<id>.<lang>` and, for long pages, `<id>-more.<lang>` after a scroll.
* Title and breadcrumb: the `st-title` header's texts must equal the registry chain (`Systeem > ... > page`, parents
  read from registry.rs, titles from `harness.settings_titles()`), the `crumb-<id>` buttons the ancestors; a data page's
  last crumb is its item (Systeem > Geluid > WinPoint Test Speakers, ... > WinPoint Test Microphone).
* No Dutch word of these pages in en (the scenario's own list, as before).
* Real pointer clicks only: `press()` first scrolls the content with the virtual pointer's wheel (Clutter
  `notify_discrete_scroll` over the content's right margin, never over a slider) until the widget is below the fixed
  header, then `h.click` (a Clutter click). Without it a widget behind the header got the header's click and one out
  of the window fell back to the dev channel's click silently.
* Every bound control flipped (`flip()`, `choose()`, `spin_up()`, radio labels) and GNOME's exact value read from the
  private keyfile; an outside write (`h.gsettings_set`) the control follows (`follows()`).

### Per page (both languages)

* Beeldscherm: GNOME path on the harness's mutter; without hardware: Helderheid shows "Alleen beschikbaar voor
  ingebouwde schermen" (no backlight), no Privacyscherm, no Draaivergrendeling, no automatic brightness, Nachtlampje's
  card says it isn't available (NightLightSupported false, as GNOME). Meerdere beeldschermen: Dit mijn hoofdbeeldscherm
  maken on and insensitive for the only display, Detecteren gives "Geen ander beeldscherm gevonden". With the sensor
  stand-in (and Settings started again for fresh caps): Draaivergrendeling shows, writes orientation-lock and follows;
  automatic brightness still needs a backlight.
* Amendment 64 through the real Schaal drop-down (125 %): applied persistently, exactly one keep question (GNOME
  Shell's DisplayChangeDialog while panelBridge.js does not route `display.*`: Notes for I 1 of slice 1; the scenario
  names who asked and takes Settings' dialog once it comes), unanswered the old scale is back after mutter's 20 s and
  the drop-down shows 100 % again.
* Nachtlampje reached by its chevron: GNOME's "Night Light Unavailable" (virtual CRTC without gamma; its controls stay
  covered by unit tests).
* Geavanceerd beeldscherm: no VRR, no Aanpassen voor tv, one rate (60 Hz) insensitive; Schaal voor apps herstellen
  (Ubuntu's override turns xwayland-native-scaling on) writes xwayland-scaling-factor 1 when turned off and follows an
  outside 0.0.
* Geluid, Eigenschappen (data page), Alle geluidsapparaten, Volumemixer on the private PipeWire: output pick, volume,
  mute, the input volume, Instellen als standaard, Volume, Rechterkanaal at half (balance -0.5, left 100, right 50),
  Volume boven 100% toestaan (flip and outside write), a click in Alle geluidsapparaten opens the input's Eigenschappen
  (Je microfoon testen), Volumemixer's output drop-down (default sink), master volume, Systeemgeluiden (event role),
  the app's volume and its Uitvoerapparaat drop-down (the stream moves).
* Energie en batterij without stand-ins: no Batterij group (the Netstroom card), Energiestand "Niet beschikbaar"
  insensitive, no Energiebesparing, automatic sleep insensitive with "Deze computer ondersteunt automatisch slapen
  momenteel niet." and no suspend warning; Scherm uitschakelen na 1 uur (idle-delay 3600, outside 600 followed), Scherm
  dimmen (idle-dim), the power button (power-button-action nothing, outside suspend followed); Slaapstand runs
  `systemctl suspend` (the run's stub, `logs/guard-calls.log`), Opnieuw opstarten asks first and Escape runs nothing.
  With UPower, power-profiles-daemon and the sensors: the battery hero and the whole Batterij group, Energiestand
  Gebalanceerd, Beste prestaties = `Set ActiveProfile 'performance'` (Properties.Set without the interactive flag, as
  GNOME: ppd's switch-profile is allow_active yes), Altijd energiebesparing gebruiken = `'power-saver'` and the
  Energiestand drop-down follows, power-saver-profile-on-low-battery, Batterijpercentage (the mirror puts it in
  settings.json taskbar.batteryPercent; the tray text is step I's, known 'I'), Batterijgezondheid behouden =
  EnableChargeThreshold(true) with interactive authorization, Batterijniveaus' Afgelopen 7 dagen = GetHistory over
  604800 s.
* Opslag and Opslaginzicht (through Opslag's chevron): both purge keys from either switch, Tijdelijke bestanden
  opschonen (flip, outside write), Opslaginzicht nu uitvoeren insensitive while both purges are off and its toast,
  14 dagen, an outside 7 dagen as its own option, Nooit (remove-old-trash-files false, the age kept), Prullenbak nu
  leegmaken: an item put in the run's own Prullenbak is gone after the question.
* Multitasking: Vensters uitlijnen (flip, outside write), taskbar.allDesktops, Alt+Tab (taskView.allDesktops and both
  GNOME switchers), Vast aantal bureaubladen with Aantal bureaubladen + (num-workspaces 5), an outside
  dynamic-workspaces true hides the number, Bureaubladen op alle beeldschermen, Actieve hoek (flip, outside write).
* Schermtijd: wellbeing.history, the chart or Screen Time Unavailable, Schermtijd vastleggen (flip, Recording
  Disabled, outside write), both break reminders, the schedule (180 s every 1800 s), Geluiden (play-sound of both
  breaks), Limiet voor schermtijd, Dagelijkse limiet + 1 uur (daily-limit-seconds 32400, written 0.5 s later),
  Grijswaarden, and an outside daily-limit-enabled false makes Grijswaarden insensitive.
* Owned keys: `gsettings.get` of all 32 keys these pages bind: none is WinPoint-owned (allow/system.rs has no Owned
  row, so no page shows a pointer card instead of a control), all available and writable, show-battery-percentage
  Mirrored to taskbar.batteryPercent.
* Mirror (plan "testing", settings-system.py's share): Niet storen in Settings sets show-banners false and the shell's
  Snelle instellingen tile is on; a real click on that tile sets show-banners true again; an outside show-banners
  false reaches settings.json notifications.dnd and the Settings switch follows.
* `--strace`: the connect and exec checks pass in both runs (no real pkexec, systemctl or loginctl; the one
  `systemctl suspend` is the stub's).

### Stand-ins (`tests/gnome/lib/standins/d.py`)

`upower(now)` (unchanged), `sensor_proxy()` (iio-sensor-proxy with an accelerometer and a light sensor: caps.get
`accelerometer`/`ambientLight`), `power_profiles()` (power-profiles-daemon 0.30 as org.freedesktop.UPower.PowerProfiles
with its three profiles, ActiveProfile writable). Profiles is an `aa{sv}`, which B0's JSON spec cannot carry, so
`start_text()` runs B0's own server with one patched function: a property given as `{"gvariant": "<text>"}` is parsed
with `GLib.Variant.parse`; the call log and records are B0's. If B0's server text changes, `start_text` fails with a
check instead of running something else.

### Page fixes this pass found (looking at the screenshots against Windows 11)

* Eigenschappen: Linkerkanaal and Rechterkanaal now line up under Volume (the mute button's room, the slider, the
  value) and show their value as Windows does (`bound_text`, test
  `channel_sliders_show_their_value_but_the_fade_does_not`); the fade keeps no number.
* Multitasking: both Bureaubladen drop-downs 360 px, so "Alleen op het bureaublad dat ik gebruik" is no longer cut off.
* Rows that belong to the row above them (Dagelijkse limiet, Grijswaarden, Schema voor bewegingspauzes, Aangepaste
  schaal, a Volumemixer app's Uitvoerapparaat) line their text up with that row's title (`sub_row`: an empty 20 px
  icon), as the rows inside a Windows expander do.
* Energie: GNOME's "Als je automatisch slapen uitschakelt ..." warning only where the pc can sleep at all
  (`suspend_off` reads canSuspend; GNOME Settings hides its automatic suspend row then); the harness's session without
  logind showed the warning under "Deze computer ondersteunt automatisch slapen momenteel niet."

### Notes for I

5. kit `native/kit/src/controls/dialog.rs` (frozen): no button of a kit Dialog takes a real pointer click. The smoke
   layer's `GestureClick` ("Clicks on the smoke stay there") runs in the bubble phase and claims every press that
   reaches it, also the ones on the dialog's own buttons, drop-downs and check boxes; claiming cancels GtkButton's
   capture-phase gesture before its release, so `clicked` never fires (Enter, Escape and the harness's dev click still
   work). Harness evidence: the known 'kit' check "a real pointer click on a kit dialog's button (Leegmaken)", a
   Clutter click at the exact centre of Prullenbak leegmaken's Leegmaken that empties nothing; slice 2 met the same in
   Meer geluidsinstellingen. It affects every confirm in WinPoint (Afmelden?, Opnieuw opstarten?, ...). Patch:

   ```rust
   // Clicks on the smoke stay there (modal); a press on the dialog itself is left to its widgets
   let block = gtk::GestureClick::new();
   block.connect_pressed(|g, _, x, y| {
       if let Some(w) = g.widget()
           && w.pick(x, y, gtk::PickFlags::DEFAULT).as_ref() == Some(&w)
       {
           g.set_state(gtk::EventSequenceState::Claimed);
       }
   });
   ```

   Then settings-system.py's check passes (drop its `known`), and `alerts()` can click the Geluiden tab's controls for
   real instead of with the dev channel.
6. settings-system.py after slice 1's note 1 (panelBridge.js routes `display.*`): the keep check takes whichever
   dialog showed and says which; once the route is in, make the GNOME Shell branch a failure (only Settings' dialog
   may ask, amendment 64).
7. settings-system.py's tray check ("the tray shows 76% next to the battery") is known 'I' until §8.2's
   `taskbar.batteryPercent` consumer is in `native/shell/src/panel/taskbar/system.rs`; then drop its `known`.
8. Harness (frozen `tests/gnome/lib/scenario.py`), optional: `Helper.click` takes a widget's centre even when it lies
   under Settings' fixed header or outside the window (then it falls back to the dev click without saying so in the
   check). settings-system.py scrolls first with the virtual pointer's wheel (`reveal()` and `SCROLL_JS`); a shared
   helper would let every batch's scenario do the same.

### Results

* `settings-system.out/` (nl, `--strace`): 218 checks, 3 failed, all known (the kit header control and the kit
  dialog click, Notes for I 4 and 5; the tray, 7); strace connect and exec OK; peak PSS 936 MiB.
* `settings-system.en.out/` (en, `--strace`): 230 checks, 3 failed, the same three known ones; 12 no-Dutch
  checks pass; strace connect and exec OK.
* Regression: native-shell.py (nl) 21 checks, 0 failed; lock-unlock.py (nl) 5 checks, 0 failed (both with this
  batch's binaries; their result dirs were not kept, they are not batch D's).
* Native unit tests (`cargo test --release`): 88 passed, 1 ignored; the settings crate 52 (one new:
  `pages::sound_device::tests::channel_sliders_show_their_value_but_the_fade_does_not`;
  `pages::power::tests::gnomes_warning_shows_while_automatic_sleep_is_off` gained the no-logind case). `checks.sh d`: 0
  errors; warnings at the base (check_rust 57, drift 15, native-dutch-only 48); the installer check refuses with rc=70
  ("XDG_RUNTIME_DIR is not 0700") as before.

### Open issues

* Nachtlampje's own controls still cannot be clicked in the harness (mutter's virtual CRTC has no gamma, so
  NightLightSupported is false, as GNOME Settings would show it too); unit tests cover them.
* Batterij opladen's Maximale lading cannot be shown sending EnableChargeThreshold(false): the UPower stand-in's
  properties do not change after a call, so the page (correctly) goes back to Maximale lading and that radio was
  already active.
* The Hyprland paths (hypr_display, the wpctl sound page) are not in this scenario; the harness is GNOME only.
* Kit dialogs and clickable headers: Notes for I 4 and 5; the harness's checks for both are known 'kit'.

## Review fixes (batch D review, 16-11)

Eight findings; each was read against the code first. All eight were real and are fixed, each with a test.

| # | Finding | Verdict | Fix | Test |
|---|---------|---------|-----|------|
| D-1 | displayBridge.js numbered its keep questions from 1 per bridge; the extension builds a new bridge at every unlock, and Settings' `ASKED` list never forgot an id, so a change after a screen lock got no keep question anywhere | real (once step I routes `display.*`) | `extension/displayBridge.js`: one module-level `nextId()`, `max(last + 1, milliseconds since boot)`: rising within the shell's lifetime across bridges, time-based across a reload. `pages/display.rs`: `OpenQuestions`, an answered question leaves the list (kept, reverted or timed out), so a reused id asks again | native `one_keep_question_per_change_and_an_answered_id_asks_again`; harness `bridge_ids()`: two DisplayBridges one after the other in the real GNOME Shell give rising ids |
| D-2 | `AudioMonitor::start` took the pulse listener whenever libpulse loads, also without a pulse server; pw-dump never started and `alive()` stayed false | real | `system/audio_monitor.rs`: the listener is registered whenever libpulse loads, pw-dump starts on the monitor's own thread unless the client connects within its first 1.5 s (`sound::audio_connected`), `alive()` = connected client or pw-dump; `start()` never waits | host `without_a_pulse_server_pw_dump_watches_the_graph`, `a_connected_pulse_client_is_the_only_source` (a stand-in `Pulse` and a pw-dump stand-in script) |
| D-3 | the UPower watcher woke 5 times a second for as long as the Battery service lived | real | `battery.rs`: no periodic tick; every threshold signal restarts a one-shot 200 ms `timeout_source_new` on the watcher's context ("200 ms after its last signal", as documented); `Drop for Battery` wakes the context so the thread still ends with the service | host `the_watcher_sleeps_between_signals_and_tells_one_burst_once` (private bus: at most 1 wakeup in 1.2 s idle, three signals 30 ms apart are one battery.changed, a Percentage change none) |
| D-4 | Linkerkanaal/Rechterkanaal wrote a pair filled once at build time; after a volume change elsewhere a one-channel drag sent the stale other side, and the host made it the volume (20 % to 100 %) | real | `pages/sound_device.rs`: `Channels` lives in `Live` and follows every volume change in `update()` (a side the user is moving keeps its own) | native `a_channel_write_keeps_the_other_channel_at_the_servers_value`; harness device page: volume 20, then Rechterkanaal, Linkerkanaal stays at 20 |
| D-5 | every reload rebuilt Meerdere beeldschermen collapsed (Detecteren, Dit mijn hoofdbeeldscherm maken, display.changed); Beeldscherminformatie on Geavanceerd beeldscherm reopened the same way | real | `pages/display.rs`: `GPage.multi_open` / `info_open`, restored on every build and kept by `connect_toggled` | harness: after Detecteren the expander still has `w11-expanded` |
| D-6 | the apps' Uitvoerapparaat/Invoerapparaat drop-downs got their options only when the stream list changed | real | `pages/volume_mixer.rs`: `targets_shape` (the sinks and sources a stream can move to); when it changes while the streams stay, every row's drop-down gets its options again before `set_selected` | native `streams_move_to_real_devices_only` extended (a new sink or source changes the key, a volume change does not); harness: a null sink created while pw-play plays is in the app's drop-down and takes the stream |
| D-7 | the Alt+Tab row is named after GNOME's app-switcher key but only read and followed taskView.allDesktops | real | `pages/multitasking.rs`: `alt_tab_choice`: the row shows GNOME's key when it is writable, follows its outside changes (`gs.follow`), and taskView.allDesktops follows it (at the page's start too, GNOME winning as at the mirror's first start, amendment 47); a locked or missing key leaves the row on taskView.allDesktops. Never a GNOME write without the user's choice. A mirror pair for while the page is closed: Notes for I 9 | native `the_alt_tab_row_follows_gnomes_app_switching_and_taskview_follows_it`; harness: outside `current-workspace-only` true and false move the row and taskView.allDesktops |
| D-8 | Dagelijkse limiet's boxes ignored lockdown and kept a refused value | real | `pages/screentime.rs`: both boxes insensitive with `gs::locked_text()` while the key is not writable (limits_group's sync() only sets the card around them); a refused write shows the key's value again (`limit_parts`, the same silent path as an outside change) | native `the_daily_limit_boxes_show_the_keys_value_again_after_a_refused_write`; lockdown itself cannot be set up in the harness (its /etc is the host's, read-only) |

### Notes for I (review fixes)

9. `host/src/settings_mirror.rs` (B0, frozen): a mirror pair for the Alt+Tab row, so taskView.allDesktops and GNOME's
   App Switching also agree while Settings is closed (D-7). Inverted, as notifications.dnd ↔ show-banners. In `PAIRS`,
   after the `taskbar.batteryPercent` pair:

   ```rust
       Pair {
           settings: &["taskView.allDesktops"],
           schema: "org.gnome.shell.app-switcher",
           key: "current-workspace-only",
           conv: Conv::Inverted,
       },
   ```

   With it, `host/src/services/gsettings/allow/system.rs` (D's) turns the app-switcher row's mode from `Write` into
   `Mirrored("taskView.allDesktops")` (amendment 49), a one-line change I makes together with the pair. The
   window-switcher key stays `Write` (GNOME Settings has no control for it; the row writes it along). Note for the
   CHANGELOG: on the first panel start GNOME's value wins (amendment 47), so WinPoint's Alt+Tab shows the windows of
   all desktops unless the GNOME session had "current workspace only"; the Multitasking page does the same alignment
   on its own until the pair is in.
10. Slice 1's Notes for I 1 builds the DisplayBridge in the extension's `enable()`, which runs again at every unlock:
   that is fine now (D-1), the ids no longer start over with a new bridge.

### CHANGELOG lines for I (batch D, System)

- **Display as in Windows 11.** Settings › System › Display arranges your displays by dragging, identifies them, and
  sets brightness, Night light, HDR, scale, resolution, orientation and refresh rate on GNOME's own display settings.
  A change asks "Keep these display settings?" once and goes back after 15 seconds without an answer. Advanced display
  shows each display's details and dynamic refresh rate.
- **Sound with every GNOME function.** Sound picks the output and input, with their volumes and a level meter;
  Properties sets a device's format, port, left and right channels and volume above 100%; All sound devices and the
  Volume mixer set every app's volume and the device it plays on. More sound settings has the alert sounds and the
  startup sound.
- **Power & battery, Storage, Multitasking and Screen time on GNOME.** Power & battery adds the power button, energy
  saver, the battery charge limit and the battery levels of the last 24 hours or 7 days; Storage Sense deletes Recycle
  Bin items and temporary files older than the period you choose, as GNOME does; Multitasking has Snap windows and
  the Desktops rows, also for what Alt+Tab shows, which GNOME's app switcher follows; Screen time shows GNOME's
  statistics, the daily limit and the break reminders.

### Results (review fixes)

* Builds (`build.sh d all`): host and native release builds clean for every file of this batch (the warnings are
  the base's), `tests/build/native-release` refreshed (19 binaries, SHA256SUMS).
* Host unit tests (`--lib`): 614 passed, 1 failed, 19 ignored (634; the review adds 3:
  `services::system::audio_monitor::tests::{without_a_pulse_server_pw_dump_watches_the_graph,
  a_connected_pulse_client_is_the_only_source}`, `services::battery::tests::
  the_watcher_sleeps_between_signals_and_tells_one_burst_once`, none skipped: the private bus was there). The one
  failure is slice 2's known `services::update::elf::tests::our_binaries_equal_readelf` (the host binaries need
  libm.so.6 for B0's `pow`; `tests/update/fixtures/compat/readelf-needed.txt` is I's to record again).
* Native unit tests (`cargo test --release`): 92 passed, 1 ignored; the settings crate 56 (the review adds 4:
  `pages::display::gnome_tests::one_keep_question_per_change_and_an_answered_id_asks_again`,
  `pages::sound_device::tests::a_channel_write_keeps_the_other_channel_at_the_servers_value`,
  `pages::multitasking::tests::the_alt_tab_row_follows_gnomes_app_switching_and_taskview_follows_it`,
  `pages::screentime::tests::the_daily_limit_boxes_show_the_keys_value_again_after_a_refused_write`;
  `pages::volume_mixer::tests::streams_move_to_real_devices_only` extended).
* `checks.sh d`: rustfmt, no-webkit, i18n, pairs, rust (57 warnings, the base's), drift (15 warnings, the base's),
  native, test_native, settings-pages and release all rc=0; the installer check refuses with rc=70 ("XDG_RUNTIME_DIR
  is not 0700") as before. No new UI string: the fixes reuse existing pairs.
* Harness, `settings-system.py` under strace: nl 224 checks, 3 failed, all known (kit header switch and kit dialog
  click, Notes for I 4 and 5; the tray percentage, 7); en 236 checks, the same 3 known; strace connect and exec OK in
  both; peak PSS 960 MiB (nl), 955 MiB (en). The review adds 6 checks per language (D-1, D-4, D-5, D-6 and D-7
  twice), all pass. The first nl run showed that the old Rechterkanaal check only passed with the stale pair of D-4:
  the Volume slider's commit landed after the 100/100 reset and took the sink back to 50/50, and with the pair now
  following the server the click wrote 50/50. The check now waits 1 s and for both channels at 100 before the click
  (the page's behaviour is the right one: the other channel keeps the server's value).

### Open issues (review fixes)

* D-7: GNOME's App Switching defaults to all workspaces, WinPoint's taskView.allDesktops to "only this desktop"
  (Windows' default). Opening Multitasking aligns taskView.allDesktops to GNOME's key, as the mirror's first start
  would (Notes for I 9), so on a fresh install WinPoint's Alt+Tab then shows the windows of all desktops. If Robin
  wants Windows' default to win instead, the pair has to go settings.json → GNOME on the first start (as the accent
  look pair does), and the page's `alt_tab_choice` would then write GNOME's key, which only a user's choice may do
  today; left as GNOME wins.
* D-2: a pulse server that goes away later (pipewire-pulse stopped) still falls back to system.rs's 2.5 s poll, as
  before; pw-dump is only chosen at the monitor's start.
* D-8: lockdown cannot be set up in the harness (its /etc is the host's, read-only), so the locked boxes are covered
  by the unit test of the value they show and by reading the code; `gs.writable` is the same test every bound control
  uses.
