# Settings parity, batch A (Input & accessibility): handoff

Contract: `notes/claude/settings-parity.md` (§1 amendments 8-10, 13-15, 24, 36, 44, 46, 49, 58, 67; §2; §4.2; §5.2).
Batch A is built in four slices in worktree `a` (branch `claude/settings-a`); each slice appends its section here (slice 4 is the last).

## Slice 1: Muis, Touchpad, Pen en tablet

### Host

* `host/src/services/input.rs`: `METHODS` = `input.tablets` (not dangerous), `input.tablet.mapButtons` (not
  dangerous: it opens GNOME Shell's OSD; the OSD writes the pad keys). Both in `host/src/api-methods/input.json`.
* `host/src/services/input/tablets.rs`: the tablets as GNOME Settings' Wacom panel finds them. udev's database
  (`/run/udev/data/c13:<minor>`, `ID_INPUT_TABLET` / `_TABLET_PAD` / `_TOUCHSCREEN`, evdev nodes only: the uevent's
  `DEVNAME=input/event*`), the ids from `/sys/dev/char/13:<minor>/device/id/` as Mutter writes them (`%.4x`), one entry
  per vid:pid; libwacom's `.tablet` files (`/usr/share/libwacom`, `/etc/libwacom` replacing a file of the same name;
  libwacom 2's `NumRings`/`NumStrips`/`NumDials`/`[Buttons]` letters and 1.x's `Ring`/`Ring2`/`Buttons`; a
  `DeviceMatch` with a kernel name is preferred, as Huion's). `kind` is `tablet`, `display` (IntegratedIn=Display) or
  `builtin` (System; an unknown I²C tablet too). A pad without a pen (ExpressKey Remote) is a tablet with
  `pen: false`. libwacom is read only when a tablet or pad exists. The monitors for `output` come from Mutter's
  DisplayConfig `GetCurrentState` (one plain call, 2 s; none outside GNOME): an extra result field `monitors`,
  documented in the fragment, so the Pen page does not depend on batch D's `display.state`.
* `styli` holds `default-<vid>:<pid>`, Mutter's stylus id for pens without a serial (amendment 9). The serials come
  from GTK on the page (GdkSeat's tools, its `tool-added` and the test canvas): the host cannot list dconf's
  `stylus/` directories, and GNOME Settings' own tool cache (`~/.cache/gnome-control-center/wacom`) is not read.
* `input.tablet.mapButtons {vidpid}`: a malformed id `bad_args input.badTablet`, an absent tablet `not_found
  input.tabletNotFound`, a tablet without pad `unsupported input.noPad`, no extension on the bus `unsupported
  input.shellUnavailable`; otherwise the bridge call `input.padOsd {vidpid}` and its answer.
* `extension/inputBridge.js`: `input.padOsd {vidpid}` finds the pad device (`Clutter.InputDeviceType.PAD_DEVICE`
  with that vendor and product id) and calls `global.display.request_pad_osd(pad, true)`, as GNOME Shell's
  `org.gnome.Shell.Wacom.PadOsd.Show` does for GNOME Settings; event `input.padOsd`.
* `allow/input.rs`: `gtk-enable-primary-paste` added to the `org.gnome.desktop.interface` Write row (Muis › "Plakken
  met de middelste muisknop", the WinPoint addition windows.json names). No other row changed.
* i18n: `host/i18n/input.tsv` 4 rows (`input.badTablet`, `.tabletNotFound`, `.noPad`, `.shellUnavailable`), their
  `key:` lines in `tests/i18n/BASELINE-CHANGES.rust-settings-a`.

### Native pages

* **Muis** (`pages/mouse.rs`), Windows order: Primaire muisknop (left-handed), Snelheid van muisaanwijzer 1-20 =
  (n - 10) / 10, Nauwkeurigheid van aanwijzer verbeteren (accel-profile default/flat), Plakken met de middelste
  muisknop (WinPoint addition); Schuiven: Inactieve vensters schuiven (shown on and fixed: GNOME always scrolls the
  window under the pointer), Schuifrichting (natural-scroll); Aanwijsstick (only while `caps.pointingstick`; the
  page rebuilds when it comes or goes): speed and acceleration; Instellingen testen (GNOME's Test Settings: primary,
  secondary, middle and double click at the current double-click time, scrolling); Gerelateerde instellingen:
  Aanvullende muisinstellingen = Windows' Eigenschappen voor Muis (Knoppen: Primaire en secundaire knop omwisselen,
  Dubbelkliksnelheid 1-10 = 1000-100 ms with the test folder, Middelste klik nabootsen; Opties voor aanwijzer:
  speed, precision, Locatie van aanwijzer weergeven wanneer ik op de Ctrl-toets druk), Muisaanwijzer.
* **Touchpad** (`pages/touchpad.rs`): the Touchpad expander (switch = send-events disabled; Touchpad ingeschakeld
  laten wanneer er een muis is aangesloten = enabled / disabled-on-external-mouse; Cursorsnelheid 1-10 = (n - 5) / 5;
  Opnieuw instellen = `gsettings.reset` of every key of the touchpad's allow row, a tick for 3 s); Bewegingen en
  interactie: Tikken (Touchpad uitschakelen tijdens het typen, one-finger tap, the two-finger tap fixed, tap and
  drag (insensitive without tapping), the lower right corner = click-method areas/fingers), Schuiven en zoomen (two
  finger / edge written together in one `gsettings.set`, Schuifrichting, Knijpen om te zoomen fixed on), Bewegingen
  met drie / vier vingers (GNOME Shell's own; the three-finger tap from tap-button-map); Instellingen testen;
  Gerelateerde instellingen: Muis.
* **Pen en tablet** (`pages/pen.rs`): the tablet (a chooser with more than one), Kies met welke hand je schrijft
  (left-handed; hidden when libwacom says not reversible); Tablet: Tabletmodus (mapping) and Hoogte-breedteverhouding
  behouden (keep-aspect) for a pen tablet, Toewijzen aan beeldscherm (output = [vendor, product, serial], Automatisch,
  a mapped display that is not connected kept as an extra choice) unless built in, Kalibreren for a pen display or a
  built-in screen (area [0,0,0,0] first, a full-screen calibrator on the mapped monitor with GNOME's four targets at
  1/8 and 7/8, the padding computed as GNOME's calibrator does; Esc or a misclick puts the old area back), Knoppen
  toewijzen (`input.tablet.mapButtons`) for a tablet with a pad; Penknoppen (a pen chooser with more than one pen):
  Knop 1/2/3 with GNOME's seven actions and Toetsaanslag toewijzen (pickers::KeyCapture into `*-keybinding`), De
  bovenste knop van mijn pen gebruiken om inkt te wissen (eraser-button-mode default/button); Pendruk: Gevoel van
  pendruk / gumdruk (GNOME's seven preset curves, nearest preset for another curve); Instellingen testen (a pressure
  canvas that also learns a new pen's serial and rebuilds the page with it); Aanraakscherm toewijzen aan beeldscherm
  per touchscreen (amendment 8); Gerelateerde instellingen: Beeldscherm, Muis. Without a tablet: "Geen tablet
  gevonden" (and the touchscreen rows). A tablet plugged in or out (`caps.changed`) rebuilds the page.
* Shared helpers in `mouse.rs` (`pub(super)`): `bind_state`, `mapped_toggle`, `mapped_checkbox` (a key whose value
  maps to on/off: accel-profile, click-method, mapping, eraser-button-mode), `scroll_options`, `test_dialog`,
  `DOUBLE`. Duplicated controls in the Eigenschappen voor Muis dialog are named `st-mouse-props-<key>`.

### plan.json controls dropped or changed

* Muis › Scrolling: Windows' "Draai aan het muiswiel om te schuiven" and "Regels om tegelijk te schuiven" are left out
  (libinput and GTK have no such setting, windows.json); "Inactieve vensters schuiven" is shown on and fixed.
* Muis › Aanvullende muisinstellingen: its description is "Knoppen, dubbelkliksnelheid en zichtbaarheid van
  aanwijzer" instead of Windows' "Aanwijzerpictogrammen en zichtbaarheid": the dialog has no Aanwijzers tab
  (cursor-theme is in no allow table and GNOME Settings 50 has no cursor theme control).
* Muis › Aanwijsstick stays on the Muis page (plan.json), not on Touchpad (windows.json's suggestion): a pointing stick
  is not a touchpad and the Muis page is always shown.
* Touchpad › Tikken: no "Gevoeligheid van touchpad" (amendments 10, 67); "Met twee vingers tikken" is ticked and fixed
  while tapping is on (libinput cannot switch it alone). `tap-button-map`, `tap-and-drag-lock`, the touchpad's
  `accel-profile`, `left-handed` and `middle-click-emulation` have no control (GNOME Settings has none either), but
  Opnieuw instellen resets them with the rest of the row.
* Touchpad › "Meer touchpadinstellingen" (Windows opens the vendor's Mouse Properties tab) is the Muis page link; the
  Instellingen testen row is GNOME's Test Settings, which covers touchpads.
* Pen › Penknoppen: Knop 3 is always shown ("Alleen op pennen met drie knoppen"): GTK gives no button count, and
  libwacom's stylus database is not read. `eraser-button-mode` is bound (Windows' row; the stylus row is `*`; GNOME
  Settings shows no control for it, but it is the schema's own key for exactly that choice).
* Pen: Windows' "Visuele effecten weergeven", "Cursor weergeven", "Aanraakinvoer negeren wanneer ik mijn pen gebruik",
  Penmenu and handwriting are left out (no GNOME function; libinput arbitrates pen and touch itself).

### Notes for I

1. `registry.rs` (frozen): the page `pen` has `needs: "tablet"`, so a computer with a touchscreen and no pen never
   reaches "Aanraakscherm toewijzen aan beeldscherm" (amendment 8). Either let `needs` take "tablet or touchscreen"
   for `pen`, or batch A's mousepointer page (Muisaanwijzer en aanraken, a later slice) repeats the touchscreen row in
   its Aanraken section; slice 4 of A decides and says so here.
2. `extension/panelBridge.js` (multi-monitor worktree): §8 note 1 already routes `input.*` to InputBridge; the bridge
   method this slice uses is `input.padOsd {vidpid}` (event `input.padOsd`).
3. `gs.rs` (frozen): `Gs` has no write of several keys at once and its cache (`store`) is private, so the touchpad's
   scroll method (two keys, one `gsettings.set`) writes through `p.run_with` and its radio follows the
   `gsettings.changed` of both keys. A `Gs::set_values(&[(&GKey, Value)], revert)` would make it optimistic like
   the other controls. Optional.
4. A switch on a `SettingsExpander` header does not toggle on a click: the header's GtkButton takes the click and
   the expander opens or closes instead (found by the harness on the Touchpad row). touchpad.rs claims the press in
   the capture phase (`header_switch`). The same pattern is in `pages/display.rs` (night light, D) and
   `pages/notifications.rs` (F). A kit-wide fix in `native/kit/src/controls/toggles.rs` (A may edit it; this slice kept
   the fix in its own file) is an additive method:
   ```rust
   /// For a switch inside a clickable parent (a SettingsExpander header): a click toggles only the switch.
   pub fn keep_clicks(&self) {
       let click = gtk::GestureClick::new();
       click.set_propagation_phase(gtk::PropagationPhase::Capture);
       click.connect_pressed(|g, _, _, _| { g.set_state(gtk::EventSequenceState::Claimed); });
       let sw = self.inner.switch.downgrade();
       click.connect_released(move |_, _, _, _| {
           if let Some(sw) = sw.upgrade() && sw.is_sensitive() { sw.set_active(!sw.is_active()); }
       });
       self.root.add_controller(click);
   }
   ```
   then `toggle.keep_clicks()` before `exp.header().set_control(toggle.widget())` in those pages, and touchpad.rs
   drops its own `header_switch`.
5. Not batch A's, seen here: `services::update::elf::tests::our_binaries_equal_readelf` fails on every fresh host
   build of B0's tree (also in worktrees b-f, none of which has this slice): `win11-app` now needs `libm.so.6` (B0's
   `settings_mirror.rs` uses `powf`), which `tests/update/fixtures/compat/readelf-needed.txt` does not record. I
   either adds `libm.so.6` to the `win11-app` line or lets the test skip it as `elf::skipped` does.
6. Not batch A's either: `services::winpoint::tests::a_program_renamed_back_by_a_rollback_is_not_a_restart` failed
   once in a full run with ETXTBSY (it copies bash and runs the copy while other test threads fork) and passed alone.
7. Harness, for I's `settings-tour.py` and the other batches' scenarios: (a) `h.gsettings_get` reads the keyfile's
   text, and GLib's keyfile backend leaves a reset key's line in the file, so a `gsettings.reset` check reads
   GSettings' view through `h.backend('settings', 'gsettings.get', ...)` (`isDefault`); (b) a real pointer click on
   a GtkButton or GtkCheckButton inside a kit `Dialog` (the Pivot tab of Eigenschappen voor Muis, its locate-pointer
   box) did not register, while a gesture on a large box in the same dialog did and page buttons do;
   settings-input.py records the miss and uses the dev channel's click there (`real=False`). Worth a look at the
   dev state's rectangles inside the dialog layer.

### Tests

* Host (`services::input::tests`, 9): libwacom files (2.x and 1.x counts, generic files skipped), the installed
  libwacom data (skips without /usr/share/libwacom), a fixture udev + /sys + libwacom tree (pen, pad, touch part,
  legacy mouse node, a laptop pen + touchscreen, a Huion matched by kernel name, an unknown tablet, an ExpressKey
  Remote, a broken entry, an /etc override), no tablet = no libwacom read, GetCurrentState decoding, DisplayConfig
  through an in-process fake on the sandbox's private session bus (one plain call, no interactive flag), mapButtons'
  refusals and its bridge call through a fake bridge, the dispatch, and every key the three pages bind being in a
  Write row at its (relocatable) path, with `disable-while-typing-timeout` in none (amendment 67).
* Native (`pages::{mouse,touchpad,pen}::tests`): the 1-20 and 1-10 speed maps (15 = 0.5, round trips), precision =
  accel-profile, the double-click steps and timing, send-events of the switch and the box, the scroll pair, the
  corner, Reset's keys, the stylus ids and paths, the pressure presets, the calibration padding, the output choices,
  the stylus actions, the control names of two schemas with `output`.
* Harness: `tests/gnome/scenarios/settings-input.py` (Muis as the plan's reference case, Touchpad, Pen without a
  tablet).

## Slice 2: Typen, Taalopties, Geavanceerde toetsenbordinstellingen, Sneltoetsen

### Host

* `host/src/services/input.rs`: `METHODS` gains `input.layouts`, `input.engines`, `input.sources.set` (dangerous),
  `input.options.set` (dangerous), `input.preview`, `input.engineSetup`, `input.sources.current`,
  `input.sources.activate`, `input.capture`, `input.capture.cancel`, `input.shortcuts`, `input.shortcuts.set`,
  `.reset`, `.resetAll`, `.custom.add`, `.custom.update`, `.custom.remove` (the writes dangerous), all in
  `host/src/api-methods/input.json`. Every read and write goes through a `gsettings::Env` of the service's own
  (`Config::default()`; a keyfile backend in the dispatch test) and `gsettings::read_item_in` / `write_values_in` with
  the method named, so B0's checks apply (types, lockdown, held accelerators, forbidden schemas). After a write that
  happened: `input.changed {what: sources|options|shortcuts}`; never in a dry run, never for a conflict answer.
* `input/layouts.rs`: xkeyboard-config's `evdev.xml` (roxmltree with its DTD allowed; a variant without languages or
  countries inherits the layout's, as GnomeXkbInfo), names translated through the UI language's `xkeyboard-config`
  .mo (Ubuntu's langpacks first; nl has none installed here, so the names stay English, as in GNOME Settings), the
  languages through iso-codes (639-3 before 639-2, also the bibliographic codes and IBus's `ja_JP`), the countries
  through ISO 3166-1 (`common_name` first). A small GNU .mo reader (both byte orders). `input.layouts` adds
  `countries: [{code, name}]` to the contract's answer (the add dialog searches by country). IBus engines from
  `/usr/share/ibus/component/*.xml`, inline engines only, `xkb:` ones left out (GNOME lists those as layouts);
  `<engines exec=...>` components (ibus-table: Python) are not run, so their engines are missing (GNOME asks the IBus
  daemon). `setup` = `ibus-setup-<engine before ':'>.desktop` installed, what launchBridge.js starts; extra fields
  `languageName`, `layout`.
* `input/sources.rs`:
  * `input.sources.set {sources}`: `[type, id]` pairs (xkb `layout[+variant]` of `[A-Za-z0-9_-]`, IBus names as §6.6),
    duplicates dropped, at most 64; written through the `Service("input.sources.set")` row. Then Ubuntu's system
    keyboard sync in a thread of its own: polkit `CheckAuthorization` of `org.gnome.controlcenter.set-keyboard` for
    this process with flags 0 (GNOME Settings' GPermission `allowed`: no prompt ever), and only when authorized
    `pkexec /usr/libexec/gnome-control-center-set-debian-keyboard <layouts> <variants> [options]` (the xkb sources in
    order, IBus ones skipped, the current xkb-options, every argument `[a-zA-Z0-9,_:-]` or no sync), pkexec through
    `tools::system_tool`. The answer has `systemKeyboard: checking|skipped`; the event `input.changed {what:
    "systemKeyboard", ok}` follows the helper. A dry run writes nothing and syncs nothing.
  * `input.options.set {lv3?, compose?}`: GNOME's choices only (`lv3:ralt_alt` is its off), null removes that kind,
    every other xkb option kept, answer `{items, options}`.
  * `input.preview {layout}`: tecla through `tools::system_tool`, detached (a thread reaps it); `unsupported
    input.noTecla` without it.
  * `input.engineSetup {id}`: the bridge's `launch.ibusSetup {engine}` (B0's launchBridge.js).
  * `input.sources.current {}` / `input.sources.activate {index}` (not in §5.2: added for the tray's input indicator,
    amendment 15): GNOME Shell's InputSourceManager through inputBridge.js.
* `input/capture.rs`: `input.capture {timeout?}` calls the bridge with `gnome::call_api_with_timeout(timeout + 5 s)`
  (timeout 1-120 s, default 30 s). The bridge answers `{keyval, mods}`; the host names the keysym with libxkbcommon
  (`dlib!`, `xkb_keysym_get_name` of `xkb_keysym_to_lower`, xkb's Prior/Next spelled Page_Up/Page_Down as GTK does)
  and answers `{accel}`, else `{disabled}` / `{cancelled}`. Any other answer shape (the `{}` a synchronous
  panelBridge.js `Call` makes of the promise) sends `input.capture.cancel` and is `unsupported`, so KeyCapture
  listens itself. GNOME Shell has no keysym-name binding, hence the host side.
* `input/shortcuts.rs`: the catalogue is `/usr/share/gnome-control-center/keybindings/*.xml` (sections merged by
  name, an entry's own schema first), `BUILTIN` (gnome-control-center 50.3's 116 entries) without them, then
  WinPoint's `task-view` / `task-manager`. Hidden entries are not listed but count for conflicts and Alles opnieuw
  instellen. `held` comes from the extension's `task-view-restore` / `logout-restore` records (the user's value, or the
  default for `user: null`) while `task-view` / `task-manager` hold `<Alt>Tab` / Ctrl+Alt+Delete. Accelerators compare
  as GNOME's do (`<Primary>` = `<Control>`, `<Mod1>` = `<Alt>`, `<Mod4>` = `<Super>`, case, Prior = Page_Up). A
  conflict answers `{conflict: {schema, key, path, id, name, accel}}` and writes nothing; `replace` takes the
  accelerator from the other shortcut first (a custom one: binding ''). A wm key with `<Alt>Tab` while Task View holds
  it is that conflict with WinPoint's row; Vervangen frees it from `task-view` first, so B0's held check lets the write
  through. Custom shortcuts at the first free `custom<N>`, removed ones reset. `reset` / `resetAll` repeat
  `gsettings::reset`'s checks for a service caller (`reset_keys`; B0 has no service reset: note for I below).
* `allow/input.rs`: unchanged in this slice (B0's rows cover every key; the custom child's row names
  `input.shortcuts.custom.update`, which add and remove use for the child's keys too).
* i18n: 10 new rows in `host/i18n/input.tsv` (`input.badSources`, `.badOption`, `.badLayout`, `.noTecla`,
  `.previewFailed`, `.badEngine`, `.badShortcut`, `.noShortcut`, `.badName`, `.badCommand`), their `key:` lines in
  `tests/i18n/BASELINE-CHANGES.rust-settings-a`.

### Extension

* `extension/inputBridge.js`: `input.capture` (a promise: `Main.pushModal(global.stage, {actionMode: NONE})`, the
  stage's `captured-event`, the first key press that is not a modifier alone, release; plain Esc, a click or the
  timeout cancel, plain Backspace disables; events `input.capture {state}`), `input.capture.cancel`,
  `input.sources.current`, `input.sources.activate`, events `input.sourceChanged` / `input.sourcesChanged`.
* `extension/shortcutBridge.js`: amendment 58, the Windows key opens Start only while settings.json's
  `start.windowsKey` is not false (read at each press; Sticky Keys as before). Amendment 13, held per accelerator:
  `<Alt>Tab` is taken from the wm keys only while `task-view` has it, and when the user gives `task-view` another
  shortcut the wm keys get their values back (`changed::task-view` → `_syncTaskView`); the same for `task-manager`
  and `logout`.

### Native pages

* **Typen** (`pages/typing.rs`): Aanraaktoetsenbord (expander) › Het aanraaktoetsenbord weergeven: Als er geen
  toetsenbord is aangesloten (false) / Altijd (true) = `screen-keyboard-enabled`, with GNOME's behaviour as a note;
  the registry's sub-pages (Geavanceerde toetsenbordinstellingen, Sneltoetsen); Gerelateerde instellingen:
  Toetsenborden (Taalopties), Toegankelijkheid van toetsenbord.
* **Taalopties** (`pages/keyboards.rs`): Toetsenborden › Geïnstalleerde toetsenborden with Een toetsenbord toevoegen,
  one card per source with its ⋯ menu (Omhoog, Omlaag, Indeling weergeven while `caps.tecla`, Opties for an IBus
  engine with a setup, Verwijderen), the empty notice, "Schakel tussen toetsenborden met Windows + Spatiebalk" from
  `switch-input-source` with two or more sources. The add dialog: Zoeken op taal of land, first the UI language's
  layouts and those of the languages already set, Meer weergeven for everything; a click adds (single-click
  activation), as does Toevoegen. The page follows `sources` (gsettings.changed) and builds itself again.
* **Geavanceerde toetsenbordinstellingen** (`pages/keyboards.rs`): Windows' plain top part (Standaardinvoermethode
  overschrijven: the sources, the chosen one moves first; Schakelen tussen invoermethoden: Voor elk app-venster ... =
  `per-window`, the link Sneltoetsen voor invoertaal → Sneltoetsen at Typen, the current keys); WinPoint additions as
  cards: Speciale tekens invoeren (Toets voor alternatieve tekens, Samenstellingstoets, an option set elsewhere kept as
  a choice of its own), Tekenherhaling (Toetsen herhalen, Vertraging voor herhalen Lang-Kort 2000-100 ms, Herhalingssnelheid
  Langzaam-Snel 110-15 ms, the test box; insensitive while repeat is off), Windows-toets (Windows-toets opent Start =
  `start.windowsKey`; Windows-toets opent het activiteitenoverzicht = `overlay-key` 'Super' / '', "Alleen
  GNOME-sessie" and what off means in WinPoint); Gerelateerde instellingen: Taalopties, Sneltoetsen, Toegankelijkheid
  van toetsenbord.
* **Sneltoetsen** (`pages/shortcuts.rs`): module docs. Names and section titles in Windows' wording from the page's
  own tables (GNOME's English descriptions are the fallback for a key a later GNOME adds), keys as key caps in
  Windows' order (Windows, Ctrl, Alt, Shift) and names (Spatiebalk, Pijl-links, Page Up). Controls are named
  `st-shortcuts-<key>` (the keys button), `-<key>-more`, `st-shortcuts-section-<id>`, `st-shortcuts-custom-*`,
  `st-shortcuts-reset-all`.

### plan.json controls dropped or changed

* Typen: Windows' Spelling, text suggestions, multilingual suggestions and Typing insights have no GNOME function
  (windows.json: GTK 4 has no system-wide spell check). Het aanraaktoetsenbord weergeven has no Nooit (GNOME has two
  values).
* Taalopties: Windows' language pack, handwriting and speech parts are not there (E's Taal en regio has the
  languages). The layout names stay English in nl (no xkeyboard-config catalogue for nl is installed; GNOME shows the
  same). GNOME's drag to reorder is Omhoog / Omlaag.
* Geavanceerde toetsenbordinstellingen: Windows' Taallijst gebruiken (aanbevolen), De bureaubladtaalbalk gebruiken,
  Opties voor de taalbalk and the Emoji panel box are left out (no GNOME function; the tray indicator is amendment 15's
  patch below). The classic Keyboard Properties dialog windows.json suggests is not a dialog here: its Tekenherhaling
  rows are cards on the page; its Knippersnelheid van cursor belongs to Tekstcursor (a later slice of A). The Toets
  voor alternatieve tekens list has "Geen" for GNOME's off (`lv3:ralt_alt`), which plan.json's list lacks.
* Sneltoetsen: GNOME's hidden entries (Super+1..9 for pinned apps, the backward switchers) stay hidden as in GNOME
  Settings. A row shows its first accelerator and "+n" for more (the tooltip has all); GNOME writes a single one, as
  this page does. Alles opnieuw instellen keeps WinPoint's two keys and the custom shortcuts (GNOME keeps its custom
  ones), which is how "except the WinPoint-held ones" is read. The plan's "Beheerd door WinPoint" info is the WinPoint
  section (editable, amendment 13) with Start openen as a link to the Windows-toets rows.

### Notes for I

8. `extension/panelBridge.js` (multi-monitor worktree): §8 note 1's routing (`method.startsWith('input.')` →
   `this._inputBridge.call(method, args)`) is not enough for `input.capture`, which answers with a promise. GJS
   prefers a synchronous `Call` over `CallAsync`, so replace `Call(method, argsJSON)` by:
   ```js
   // D-Bus success values are raw results. Only failures carry an {ok:false,error} envelope. A bridge may answer
   // with a promise (inputBridge.js input.capture waits for a key): the reply waits for it.
   CallAsync([method, argsJSON], invocation) {
       const reply = value => invocation.return_value(new GLib.Variant('(s)', [JSON.stringify(value)]));
       const fail = error => reply({ok: false, error: {code: error?.code || 'native_error',
           message: error?.message || String(error)}});
       let result;
       try {
           const args = JSON.parse(argsJSON || '{}');
           if (!args || typeof args !== 'object' || Array.isArray(args))
               failure('bad_args', this._t('argsObject'));
           result = this._call(method, args);
       } catch (error) {
           fail(error);
           return;
       }
       Promise.resolve(result).then(reply, fail);
   }
   ```
   Until then the host sends `input.capture.cancel` on the `{}` and KeyCapture listens in the window (a grab on the
   stage is then never left behind for longer than that one round trip). Bridge methods of this slice:
   `input.capture`, `input.capture.cancel`, `input.sources.current`, `input.sources.activate`; events
   `input.capture`, `input.sourceChanged`, `input.sourcesChanged` (all `input.*`, relayed by B0's gnome.rs).
9. `gsettings.rs` (frozen): there is no service-side reset (`gsettings::reset` takes `Caller::Api` only, which refuses
   `Service` rows). `services/input/shortcuts.rs` `reset_keys` repeats its checks for a service caller. A
   `gsettings::reset_values(method, schema, path, keys, lang, dry_run)` next to `write_values` (the same `plan`
   checks with `Caller::Service(method)`, then `reset` + `sync`) would let `reset_keys` go.
10. Amendment 15, the tray's input indicator: a new `native/shell/src/panel/taskbar/input.rs`, `mod input;` in
    `native/shell/src/panel/taskbar.rs` and the button in the tray before the system button. Windows shows the
    current keyboard's short name (ENG) left of the system icons when more than one keyboard is set; a click opens the
    list. Patch:
    ```rust
    //! The tray's input indicator (notes/claude/settings-parity.md amendment 15, batch A's patch): the current input
    //! source's short name (GNOME Shell's, in capitals, as Windows' "ENG") while more than one is set; a click lists
    //! them and switches (input.sources.current / input.sources.activate through inputBridge.js); it follows
    //! input.sourceChanged and input.sourcesChanged.
    use super::util;
    use crate::core::Shell;
    use gtk::prelude::*;
    use serde_json::{Value, json};
    use std::cell::RefCell;
    use std::rc::Rc;

    pub struct Input {
        shell: Rc<Shell>,
        pub button: gtk::Button,
        label: gtk::Label,
        state: RefCell<Value>,
    }

    pub fn build(shell: &Rc<Shell>, strip: &gtk::Widget) -> Rc<Input> {
        let button = gtk::Button::new();
        button.add_css_class("w11-tb-hit");
        button.add_css_class("w11-tb-input");
        button.set_widget_name("tb-input");
        let label = gtk::Label::new(None);
        button.set_child(Some(&label));
        button.set_visible(false);
        let input = Rc::new(Input { shell: shell.clone(), button, label, state: RefCell::new(Value::Null) });
        let (weak, strip) = (Rc::downgrade(&input), strip.clone());
        input.button.connect_clicked(move |b| {
            if let Some(i) = weak.upgrade() {
                i.menu(&strip, util::center_x(b, &strip));
            }
        });
        for event in ["input.sourceChanged", "input.sourcesChanged"] {
            let weak = Rc::downgrade(&input);
            shell.on(event, move |_, _| {
                if let Some(i) = weak.upgrade() {
                    i.reload();
                }
            });
        }
        input.reload();
        input
    }

    impl Input {
        fn reload(self: &Rc<Self>) {
            let weak = Rc::downgrade(self);
            self.shell.call("input.sources.current", json!({}), move |r| {
                if let Some(i) = weak.upgrade() {
                    *i.state.borrow_mut() = r.unwrap_or(Value::Null);
                    i.render();
                }
            });
        }

        fn sources(&self) -> Vec<Value> {
            self.state.borrow()["sources"].as_array().cloned().unwrap_or_default()
        }

        fn render(&self) {
            let sources = self.sources();
            let current = self.state.borrow()["current"].as_i64().unwrap_or(-1);
            let now = sources.iter().find(|s| s["index"].as_i64() == Some(current));
            self.button.set_visible(sources.len() > 1);
            self.label.set_text(&now.and_then(|s| s["short"].as_str()).unwrap_or("").to_uppercase());
            let name = now.and_then(|s| s["name"].as_str()).unwrap_or("");
            let hint = self.shell.t("Wisselen met Windows-toets + spatiebalk", "Switch with Windows key + Space");
            self.button.set_tooltip_text(Some(&format!("{name}\n{hint}")));
        }

        fn menu(&self, strip: &gtk::Widget, x: i32) {
            let current = self.state.borrow()["current"].as_i64().unwrap_or(-1);
            let items = self
                .sources()
                .into_iter()
                .map(|s| {
                    let index = s["index"].as_i64().unwrap_or(-1);
                    let mark = if index == current { "✓  " } else { "    " };
                    let label = format!("{mark}{}", s["name"].as_str().unwrap_or(""));
                    let shell = self.shell.clone();
                    util::item(label, move || shell.fire("input.sources.activate", json!({ "index": index })))
                })
                .collect();
            util::popup_menu(strip, x, items);
        }
    }
    ```
    and in `taskbar.rs`: `mod input;` with the other modules, and where the tray is built
    `let input = input::build(shell, &strip);` with `input.button.upcast_ref()` first in the `for w in [...]` list
    that appends `sys.sys_btn` and `sys.clock_btn` (the `Rc<Input>` kept in `Bar` next to `sys`). CSS for
    `native/shell/css/taskbar.css`: `.w11-tb-input { padding: 0 8px; min-width: 36px; } .w11-tb-input label {
    font-size: 12px; }`. Two glossary rows for `tests/i18n/windows-glossary/a.tsv` with it: "Switch with Windows key
    + Space" / "Wisselen met Windows-toets + spatiebalk" (winpoint, no). The shell is the panel host's client, which
    serves `input.*` (services/mod.rs) and relays the bridge's `input.*` events (gnome.rs).
11. `gs.rs` (frozen): `Gs::load`'s `gsettings.changed` subscription holds the `Gs` only weakly, so a page whose
    controls hold no clone of it (Taalopties: its cards are built from the value, its writes go through
    `input.sources.set`) loses every `gs.follow` callback when the builder returns; the first harness run of this
    slice saw Taalopties stay as it was after a write. keyboards.rs keeps a clone until the page goes
    (`p.on_cleanup`). A fix in gs.rs: `p.on_cleanup` with a clone in `Gs::load` itself (the cycle Page → cleanup →
    Gs → Page ends when the page's cleanups run), or a sentence in its docs. A host test
    (`a_write_of_another_service_reaches_gsettings_changed`) shows that a write through `gsettings::write_values`
    from another service does reach the gsettings worker's watch.
12. `common.rs` (frozen) / KeyCapture: the host capture holds the pointer too (a stage grab), so a click on the
    dialog's Annuleren cancels the capture through the bridge (inputBridge.js treats any click as Annuleren) and the
    dialog then closes with the host's `{cancelled}`; KeyCapture needs no change.

### Tests

* Host (`services::input::keyboard_tests`, 21 new): the .mo reader (both byte orders, truncated files), the xkb
  rules (variant inheritance, ids `input.sources.set` takes) on a fixture and on the installed evdev.xml, translated
  languages and countries from fixture iso-codes and catalogues (langpack first), IBus components (inline engines,
  `xkb:` and `exec` skipped, textdomain, setup desktop files), the sources and options parsers and the helper's argv,
  the writes on a memory backend (dry run writes nothing; xkb-options keeps the other options), polkit through an
  in-process fake on the private session bus (subject unix-process of this pid, flags 0, a challenge is no), the
  helper through pkexec's place (argv), engineSetup and the indicator through fake bridges, keysym names
  (libxkbcommon, Page_Up), capture shapes and the 5 s slack, the release of a bridge that cannot wait, accelerator
  comparison, the catalogue equal to the installed files, held from the restore records with the extension's real
  compiled schema, conflicts and Vervangen (also against WinPoint's Task View), reset, resetAll keeping WinPoint's
  keys, custom add/conflict/update/remove, every key the keyboard pages bind in its row and mode, and the dispatch on
  a keyfile backend (events, no event for a conflict, a dry run writes and announces nothing), and a write through
  `gsettings::write_values` heard by the gsettings worker (`gsettings.changed`).
* Native: `pages::{typing,keyboards,shortcuts}::tests` (the touch keyboard's values, the catalogue and the add
  dialog's suggestions and search, the repeat sliders' round trips, the xkb options, the sources' JSON, every
  visible shortcut named in both languages, key caps in Windows' words and order, the search).
* Harness: `tests/gnome/scenarios/settings-input.py` parts 6-9 (Typen, Taalopties with the add dialog, Geavanceerde
  toetsenbordinstellingen, Sneltoetsen with Super+Q by real keys through a Clutter virtual keyboard).

## Slice 3: Toegankelijkheid, Tekstgrootte, Visuele effecten, Contrastthema's, Muisaanwijzer en aanraken, Tekstcursor

### The text scale: what GTK really does (contract §4.2 corrected)

The contract (§4.2, amendment 49) assumes that once text-scaling-factor carries the size, "GTK scales win11-calendar
already", so the kit, the shell and the calendar can simply stop multiplying textScale. Measured on this machine's GTK
4.22.4 (a private broadway display, `GtkSettings:gtk-xft-dpi` set to 96 and to 144 dpi): a label with `font-size:
20px` stays 28 px high at both, a label with `font-size: 15pt` goes from 28 to 41 px. GTK scales point sizes (and the
default font, which is in points) by text-scaling-factor, never px. The kit's type ramp (`--fs-*`, tokens.css), the
shell's and the calendar's are all px, so "stop multiplying" alone would have left WinPoint's text unscaled.

A second finding, from a probe in the private harness GNOME (a GTK 4 window printing its GtkSettings): GTK 4.22's
Wayland backend takes its settings from the settings portal only. Without a portal (the harness has none) it reports
`gtk-xft-dpi` -1, `gtk-interface-contrast` unsupported and the font "Sans 10", whatever the keyfile holds, and logs
"Failed to read portal settings". In a GNOME session xdg-desktop-portal-gnome serves them; in another session it
depends on the portals that run.

What slice 3 does instead, in the files §4.2 gives A: the kit (`native/kit/src/theme.rs` `System`, used by ctx.rs
`apply_look`), the shell (`native/shell/src/theme.rs`) and the calendar (`native/calendar/src/theme.rs`) read GNOME's
`org.gnome.desktop.interface text-scaling-factor` themselves (a `gio::Settings` of their own, followed by its
`changed` signal; GTK's `gtk-xft-dpi / (96 x 1024)` is the next source, settings.json's textScale the last) and scale
their px ramp by it. So text is scaled exactly once and by the same factor as GNOME's apps; a change made in GNOME
Settings, on the Tekstgrootte page or by the mirror reaches every WinPoint app at once, also in a GNOME session where
no mirror runs, and also without a portal. `kit::ctx().text_scale()` is that factor (taskmgr's `layout::fs` reads the
scaled px tokens, as before). The fallback range is GNOME's 50-300 % (was 100-225); config.rs is untouched (E's
line), its clamped value is the calendar's fallback. High contrast and the On/Off Shapes are read the same way
(`org.gnome.desktop.a11y.interface`, GTK's settings next).

Raw `font-size: Npx` rules outside the ramp (98 in the apps' CSS files, none of them A's) were never scaled and still
are not; that is unchanged, and a matter for those files' owners.

### Host

* `host/src/settings_mirror/render.rs` (amendments 46, 48, 49), the rendering pairs:
  * `accessibility.highContrast` ↔ `org.gnome.desktop.a11y.interface high-contrast` (`Conv::Bool`);
  * `accessibility.reduceMotion` ↔ `org.gnome.desktop.interface enable-animations` (`Conv::Inverted`) and ↔
    `org.gnome.desktop.a11y.interface reduced-motion` (`Conv::ReducedMotion`): GNOME Settings 50 writes reduced-motion
    (its binary has `reduced_motion_row` and `G_DESKTOP_REDUCED_MOTION_REDUCE`), GNOME Shell and GTK read
    enable-animations; a change of either GNOME key reaches the setting, the settings change writes the other key.
    enable-animations comes first in the list, so at a start where both moved, GNOME Settings' own key decides
    (start_plan's patch takes the later pair);
  * `accessibility.textScale` ↔ `org.gnome.desktop.interface text-scaling-factor` (`Conv::Percent`, no clamp: 0.5-3.0 =
    50-300 %), but only once B0's `Conv::Percent` hands an integral percentage back as an integer (`TEXT_SCALE_READY`,
    note 13). Today it gives GNOME's 1.0 back as the float 100.0, which serde_json never equals to settings.json's
    integer default 100: every start would rewrite textScale and B0's test
    `the_start_adopts_gnome_for_every_preference_pair` ("GNOME's defaults are WinPoint's defaults") fails. With the
    pair held back the kit is not affected (it reads GTK's factor, above); settings.json's textScale simply stays as it
    is until I applies note 13, and the pair then joins by itself.
  * `PAIRS` is a `LazyLock<Vec<Pair>>` (B0's `all_pairs()` takes `render::PAIRS.iter()`, which works unchanged).
  * `effects()`: amendment 46. While `accessibility.highContrast` is true, the Fluent import block leaves
    `~/.config/gtk-4.0/gtk.css` (`gtk4_remove`, the user's own CSS stays); when it goes from on to off (the last value
    this host saw, so no other settings change ever writes gtk.css) the block comes back where WinPoint's look puts it
    (theme.syncSystem, mode light or dark, gtkFamily not "system": `sync_gtk4_block("fluent")`). WinPoint's default
    gtkFamily is "system", where there is no block either way. Two gaps sit in frozen settings_effects.rs: note 14.
* No `input.*` method in this slice: the pages bind GNOME's keys through `gsettings.*`. `services/input/a11y_tests.rs`
  checks every key they and the quick settings tile read or write against its allow row (Write; the four Mirrored
  rows with their settings; toggle-contrast `Service("input.shortcuts.set")`; cursor-theme in no table).
  `allow/input.rs`: unchanged (B0's rows cover every key).

### Native: kit, shell, calendar

* `native/kit/src/theme.rs`: `System {text_scale, high_contrast, status_shapes}` read from GNOME's keys
  (text-scaling-factor, high-contrast, show-status-shapes; the two schemas opened once in `install`, where installed)
  and else from GtkSettings (gtk-xft-dpi, gtk-interface-contrast, gtk-show-status-shapes), `Look::from_settings_and_system`
  (GNOME's text scale replaces textScale, GNOME's high contrast joins accessibility.highContrast for `w11-hc`, i.e. the
  kit honours GNOME's high contrast as GTK's `prefers-contrast: more` does, amendment 46), `Look.status_shapes`;
  `install` re-applies the look on the keys' `changed` and on GTK's three notifies.
  GNOME's On/Off Shapes: GtkSwitch draws its own I and O images only while GTK's `gtk-show-status-shapes` is on, and
  GTK takes that from the portal alone, so `apply` sets it from GNOME's key (as it sets gtk-enable-animations), and the
  dynamic CSS shows the images in the kit's colours (controls.css hides them otherwise); `controls/toggles.rs` needed
  no change. Additive: `Look::from_settings` keeps its signature (range 50-300 %), the
  new field has a default; nothing outside the kit builds a `Look`.
* `native/kit/src/ctx.rs`: `apply_look` uses `from_settings_and_system(.., System::read())`; `text_scale()` and
  `high_contrast()` docs; new `status_shapes()`.
* Reduced motion stays settings.json's (mirrored from both GNOME keys): GTK's gtk-interface-reduced-motion is not read,
  because the kit sets gtk-enable-animations itself and a value GTK derived from it would hold the look on.
* `native/shell/src/theme.rs`: the same (`look_from`, `system()`, the keys' signals, gtk-show-status-shapes), the
  I/O shapes for `switch.q-switch`.
* `native/calendar/src/theme.rs`: the ramp by GNOME's factor (`text_scale`), the last `apply` kept to re-apply when
  the key changes; config.rs unchanged.
* `native/shell/src/flyouts/quick.rs`: it never multiplied a text scale (its text sizes are the shell theme's), so
  nothing to stop there. New: the Toegankelijkheid tile (GNOME's accessibility menu, plan.json's "tegel in Snelle
  instellingen"): shown while `always-show-universal-access-status` is on or one of its functions is (GNOME's rule),
  lit while a function is on; a click (or its >) opens its page with GNOME's menu in Windows' words: Vergrootglas,
  Hoog contrast, Verteller, Plaktoetsen, Filtertoetsen (slow keys), Muistoetsen, Schermtoetsenbord, each a row with a
  switch (`gsettings.get` / `gsettings.set` on the panel host, optimistic, back with the host's error or in test mode,
  following `gsettings.changed`), and Meer toegankelijkheidsinstellingen. The tiles now carry widget names
  (`qs-tile-<id>`, `qs-tile-<id>-main`) and the rows `qs-a11y-<key>`.

### Native pages

* **Toegankelijkheid** (`pages/accessibility.rs`): Windows' Zicht, Gehoor, Interactie (B0's links); Snel aanpassen:
  Animatie-effecten (below) and Toegankelijkheidsmenu altijd weergeven (windows.json's WinPoint addition at the bottom;
  "De tegel Toegankelijkheid weergeven in Snelle instellingen").
* **Tekstgrootte**: Voorbeeld van tekstgrootte (the sample at the chosen size, absolute px whatever the page's own
  scale), Tekstgrootte 100-225 % in steps of 1 with Toepassen = `text-scaling-factor` (percent / 100; Mirrored). A value
  outside Windows' range (GNOME's schema has 0.5-3.0) is shown as it is ("80%") and written only by Toepassen; the
  slider follows a change from elsewhere unless it is in use.
* **Visuele effecten**, Windows' order: Schuifbalken altijd weergeven (`overlay-scrolling`, inverted), Transparantie-
  effecten (theme.transparency), Animatie-effecten (reduced-motion and enable-animations written together, so it also
  holds where no mirror runs; the mirror keeps reduceMotion), Vormen voor aan/uit-status (`show-status-shapes`);
  Gerelateerde instellingen: Contrastthema's, Tekstgrootte.
* **Contrastthema's**: Windows' sentence, the drop-down Geen / Hoog contrast with Toepassen (`high-contrast`,
  Mirrored), Sneltoets voor contrastthema's (GNOME's `toggle-contrast` as key caps or Uitgeschakeld; a click opens
  Sneltoetsen at Toegankelijkheid), the note that GNOME and GTK apps have one high-contrast style; WinPoint's tips
  (Licht of donker, Transparantie uitzetten, Tekstgrootte) stay.
* **Muisaanwijzer en aanraken** (`pages/a11y_vision.rs`): Muisaanwijzer: Grootte on GNOME Settings' five sizes 24
  (standaard), 32, 48, 64, 96 px (`cursor-size`; a size set elsewhere shows at the nearest step, written only when the
  slider moves), Muisaanwijzerindicator (`locate-pointer`); Aanraakscherm: pen.rs' touchscreen rows (now
  `pub(super)`) for every touchscreen, from `input.tablets`; Gerelateerde instellingen: Muis.
* **Tekstcursor**: Knipperen: Knipperende tekstcursor (`cursor-blink`), Knippersnelheid van cursor
  (`cursor-blink-time` 100-2500 ms, Langzaam left, Snel right; insensitive while the cursor does not blink, as GNOME
  Settings), Testvak (an entry: GTK follows the keys at once); Gerelateerde instellingen: Geavanceerde
  toetsenbordinstellingen. Vergrootglas and Verteller stay B0's placeholders for slice 4.

### plan.json controls dropped or changed

* Visuele effecten: Windows' "Meldingen sluiten na deze tijd" is left out (GNOME Shell's banner time is fixed, and a
  WinPoint toast timeout would need a new settings-defaults.json key, which is frozen).
* Contrastthema's: Windows' four palettes (Aquatisch, Woestijn, Schemering, Nachtelijke hemel) and Bewerken have no
  GNOME function (GTK has one high-contrast style): Geen and Hoog contrast only. Windows' shortcut (Left Alt + Left
  Shift + Print Screen) is GNOME's toggle-contrast, unset by default: shown, set in Sneltoetsen.
* Muisaanwijzer en aanraken: Stijl van muisaanwijzer (cursor themes, plus Windows' colours) is left out:
  GNOME Settings 50 has no such control and cursor-theme is in no allow table. Windows' 15 size steps are GNOME's 5.
  Windows' Aanraakindicator has no GNOME function; its place holds GNOME's touchscreen mapping. This settles note 1:
  a computer with a touchscreen and no pen reaches the mapping here, so pen's `needs: "tablet"` can stay.
* Tekstcursor: Windows' Indicator voor tekstcursor and Dikte van tekstcursor have no GNOME function (windows.json):
  left out; the page holds GNOME's Cursor Blinking, in the classic Keyboard Properties' words (Knippersnelheid van
  cursor).
* Toegankelijkheid: "Snel aanpassen (ongewijzigd)" keeps its Animatie-effecten card, now bound to GNOME's keys
  (amendment 49) instead of settings.json.

### Notes for I

13. `host/src/settings_mirror.rs` (frozen), `Conv::Percent` (amendment 48): `to_winpoint` rounds and gives a float, so
    GNOME's 1.0 becomes 100.0, never equal to settings.json's integer default 100 (every start rewrites textScale, B0's
    defaults test fails), and a GNOME value with more than two decimals comes back rounded and is written back to GNOME.
    Patch, both in settings_mirror.rs:
    ```rust
    // to_winpoint
    Conv::Percent => {
        // an integral percentage as an integer (settings.json's default is 100); any other value unrounded, so
        // GNOME's own value comes back (amendment 48)
        let percent = value.get::<f64>()? * 100.0;
        if (percent - percent.round()).abs() < 1e-9 { json!(percent.round() as i64) } else { json!(percent) }
    }
    // gnome_writes, in its filter_map: compare in WinPoint's domain, so GNOME's value that maps to the setting
    // already (also one the conversion cannot write back bit for bit) is no write
    let path = touched(pair, changed)?;
    let target = to_gnome(pair.conv, at(settings, path))?;
    let current = gnome.read(pair.schema, pair.key)?;
    (to_winpoint(pair.conv, &current).as_ref() != Some(at(settings, path))).then_some((pair, target))
    ```
    B0's round-trip test then expects `json!(125)` for 1.25. render.rs' `text_scale_ready()` turns true, and the text scale pair joins `PAIRS` with no change in
    render.rs; its test `the_rendering_pairs_are_the_contracts` follows either way.
14. `host/src/settings_effects.rs` (frozen), amendment 46's two gaps: `captions_at_start` adds the Fluent block at
    every panel start (before the mirror starts, and no settings change then reaches `render::effects`), and
    `Effects::apply` runs the mirror (with render's effect) before its own theme sync, so a theme change while high
    contrast is on puts the block back. Patch: treat high contrast as the "system" family in both places:
    ```rust
    fn gtk4_family(settings: &Value) -> &str {
        if settings["accessibility"]["highContrast"] == true { "system" } else { family(&settings["theme"]) }
    }
    ```
    and `sync_gtk4_block(gtk4_family(settings), ..)` in `apply` (with `"accessibility" | "accessibility.highContrast"`
    among the paths that run it) and in `captions_at_start`. render.rs' effect can stay (it is idempotent).
15. `native/shell/src/desktop/grid.rs` (multi-monitor worktree), §4.2's handoff patch: the cells grow with the text
    scale the labels have, which is now GTK's factor; in `compute_metrics` replace the settings line by
    ```rust
    let ts = crate::theme::look().text_scale;
    ```
    (`look()` is refreshed on settings.changed and when text-scaling-factor changes; the label CSS is the shell
    theme's ramp). Until then the grid's cells follow settings.json's textScale, which the held-back text scale pair
    does not update yet (note 13), so with GNOME's text size above 100 % the labels can outgrow their cells.
16. `tests/i18n` / registry (frozen): the registry description of `visualeffects` ("Animaties en transparantie")
    could follow Windows' "Schuifbalken, transparantie, animaties" now that the page has scrollbars and shapes;
    `textsize` could add the keywords "tekstschaal" / "text scaling". Optional.
17. Contract §4.2's sentence on the calendar ("GTK scales win11-calendar already") is not what GTK 4.22 does (above);
    the CHANGELOG line for the text scale should say WinPoint's apps follow GNOME's Tekstgrootte (text-scaling-factor)
    directly, also in GNOME and Ubuntu sessions. The second finding (GTK 4.22 on Wayland reads its settings from the
    portal only) matters for every batch's harness scenario: GTK apps in the harness see none of the keyfile's GNOME
    settings (font, colour scheme, contrast, text scale); only code that reads GSettings itself does.
18. Harness, for every batch: the harness's caps read the host's `/run/udev/data`, so a device of the machine
    that runs it can come and go during a run (here a touchscreen appeared in caps in one of five runs). A page that
    rebuilds itself on every `caps.changed` (slice 1's `pen.rs` does) can lose a real click that lands while it
    rebuilds; Muisaanwijzer en aanraken refreshes only when caps' touchscreen value changes. pen.rs could do the same
    for tablet and touchscreen (A's file; left as it is in this slice).

### Tests

* Host: `settings_mirror::render::tests` (5): the pairs and their order, the text scale pair present exactly when
  `Conv::Percent` keeps integral percentages integers, no clamp over GNOME's 0.5-3.0; reduced motion writes both GNOME
  keys, either brings the setting back, no loop (memory backend, a temporary settings store); a reduced motion set in
  a GNOME session comes in at the start and enable-animations follows; high contrast both ways; the Fluent block out
  of a temporary gtk.css while high contrast is on (the user's CSS stays), back only on the way off and only with
  WinPoint's Fluent family, and only changes that touch the setting run it. `services::input::a11y_tests` (1): every
  key the six pages and the quick settings tile bind has its row in the mode it needs, cursor-theme in none.
* Native: `pages::accessibility::tests` (3: reduced-motion inverted, text size in percent with GNOME's values outside
  Windows' range, contrast choices and toggle-contrast), `pages::a11y_vision::tests` (2: GNOME's five pointer sizes
  and the nearest step, the blink rate slow to fast), `kit::theme::tests` (2: GNOME's factor once, settings.json's
  only as the fallback, 21 px at 150 %; GNOME's high contrast and shapes in the look and the CSS),
  `shell::theme::tests` (1), `shell::flyouts::quick::tests` (1: the tile shows as GNOME's menu does, every row has its
  texts), `calendar::theme::tests` (1).
* Harness: `tests/gnome/scenarios/settings-a11y.py` (module docs list the checks). The text scale, the plan's risk
  "text scaling may double between GTK and the kit": a card title of the kit's px ramp is 20 px high at
  text-scaling-factor 1.0 and 30 px at 1.5 (x1.5: once). What the runs found and this slice fixed: (1) GTK's
  portal-only settings (above; the ratio was 1.0 before the kit read GNOME's keys itself); (2) the switches showed no
  I and O with On/Off Shapes on, because GtkSwitch follows GTK's own gtk-show-status-shapes, which only the portal
  sets (the kit and the shell now set it from GNOME's key); (3) the pointer size slider's first click landed a step
  off or not at all: the size label next to it changed width with the size ("24 px (standaard)", "48 px") and moved
  the slider under the pointer, and the page rebuilt itself on any `caps.changed` (now a fixed-width label, and a
  refresh only when caps' touchscreen changes; Tekstgrootte's value label is sized in characters for the same reason).
* Results of this slice: host `--lib` 609 passed, 1 failed, 19 ignored (the failure is B0's `elf::our_binaries_equal_readelf`,
  libm.so.6, note 5; slice 2 had 603 passing); native `cargo test --release` 95 passed, 1 ignored (slice 2: 85);
  settings-a11y.py nl 53/53 and en 59/59 checks (no Dutch word in English, no pkexec or tecla call in
  guard-calls.log); checks.sh as at base (installer rc 70: this shell's XDG_RUNTIME_DIR).

## Slice 4: Vergrootglas, Verteller, Audio, Toetsenbord, Muis (Toegankelijkheid)

### Host

* `host/src/services/input/screen_reader.rs`: `input.screenReader.preferences {}` (not dangerous; not in §5.2, the
  method amendment 36 asks for: "Call it from the host"). GNOME Settings 50's Screen Reader Configure
  (`configure_screen_reader_activated_cb`) calls `ShowPreferences` of the running Orca on the session bus
  (`org.gnome.Orca.Service`, `/org/gnome/Orca/Service`, interface `org.gnome.Orca.Service`; Orca 50.3's
  `orca/dbus_service.py`, installed here); this does the same: one plain call, `NO_AUTO_START` (Orca's name is not
  activatable; GNOME Shell starts Orca when `screen-reader-enabled` turns on), no interactive-authorization flag, a
  10 s timeout (Orca builds its window before it answers). `{shown: true}`; nobody owning the name (ServiceUnknown or
  NameHasNoOwner) `unsupported input.orcaNotRunning`; Orca answering false (no script) or any other error `failed
  input.orcaFailed`. Orca's window is Orca's own process (Python, GTK 3): nothing of it runs under WinPoint, and
  launchBridge.js is not used for Orca. In `METHODS` and `host/src/api-methods/input.json`.
* i18n: `host/i18n/input.tsv` 2 rows (`input.orcaNotRunning`, `input.orcaFailed`), their `key:` lines in
  `tests/i18n/BASELINE-CHANGES.rust-settings-a`.
* `allow/input.rs`: unchanged. B0's rows cover every key of the five pages: the a11y applications, magnifier (each
  key named), `a11y.keyboard` and `a11y.mouse` (`*`) and wm.preferences rows are Write; media-keys `magnifier`,
  `magnifier-zoom-in`, `magnifier-zoom-out`, `screenreader` and shell keybindings `show-screenshot-ui` are
  `Service("input.shortcuts.set")`; `allow-volume-above-100-percent` is batch D's Write row (one owner, amendment 50).
  `audible-bell` stays in no table (windows.json: "remains").

### Native pages

* **Vergrootglas** (`pages/a11y_vision.rs`), Windows' order: Vergrootglas (expander, its switch on the header =
  `screen-magnifier-enabled`; inside GNOME's three magnifier shortcuts as key caps: Vergrootglas in- of uitschakelen,
  Inzoomen, Uitzoomen, each a link to Sneltoetsen at Toegankelijkheid); Zoomniveau with − and + and the level in %
  (`mag-factor`, GNOME Settings' 1.00-20.00 in quarters; a factor set elsewhere steps onto the next quarter);
  Kleuren omkeren (`invert-lightness`). Weergave: Weergave Volledig scherm / Vastgezet / Lens (`lens-mode` and
  `screen-position` in one `gsettings.set`), Schermgebied (the half; insensitive at Volledig scherm), Buiten de
  schermranden doorgaan (`scroll-at-edges`), De muisaanwijzer houden (expander, radio: Gecentreerd op het scherm =
  centered, Binnen de randen van het scherm = push, Met de inhoud meebewegen = proportional), Vergrootglas laten
  volgen (expander: Muisaanwijzer = mouse-tracking none or the chosen way, Toetsenbordfocus = `focus-tracking`,
  Tekstcursor = `caret-tracking`). Kleur en dradenkruis: Kleuraanpassing (Helderheid and Contrast write their red,
  green and blue keys together, -0.8..0.8; Kleur = `color-saturation`), Dradenkruis (switch on the header =
  `show-cross-hairs`; Over muisaanwijzer heen = `cross-hairs-clip` inverted, Dikte 1-100 px, Lengte 0-100 % = 20-4096
  px, Kleur through the colour picker, Dekking = `cross-hairs-opacity`). Gerelateerde instellingen: Tekstgrootte,
  Muisaanwijzer en aanraken.
* **Verteller** (`pages/a11y_vision.rs`): Verteller (expander, switch = `screen-reader-enabled`, open; inside
  Sneltoets voor Verteller = media-keys `screenreader` as key caps, and a line that Verteller is Orca); Stem,
  uitgebreidheid en braille: Instellingen voor Verteller openen (`input.screenReader.preferences`; sensitive while the
  screen reader is on, as GNOME's row; "Orca ... is niet geïnstalleerd" without `caps.orca`). Gerelateerde
  instellingen: Vergrootglas, Toetsenbord.
* **Audio** (`pages/a11y_interaction.rs`): Mijn scherm laten knipperen tijdens audiomeldingen (Uit / Het actieve
  venster laten knipperen / Het hele scherm laten knipperen = `visual-bell` + `visual-bell-type`, together) with
  Testen (GNOME's Test Flash: `gdk_surface_beep` of the window, which Mutter turns into the visual alert);
  Overversterking (`allow-volume-above-100-percent`). Gerelateerde instellingen: Geluid.
* **Toetsenbord** (`pages/a11y_interaction.rs`): Plaktoetsen, Filtertoetsen en Wisseltoetsen as three expanders with
  their switches: Plaktoetsen (`stickykeys-enable`; Sneltoets voor Plaktoetsen; the two check boxes; amendment 14's
  note), Filtertoetsen (on while Slow Keys or Bounce Keys is on; its switch writes `slowkeys-enable` true, or both
  false; Sneltoets voor Filtertoetsen; Korte toetsaanslagen negeren (Trage toetsen) with Acceptatievertraging 0-500 ms
  and the three beeps; Onbedoelde toetsaanslagen negeren (Stuitertoetsen) with 0-900 ms and the reject beep; the
  delays and beeps insensitive while their feature is off; Tekenherhaling → Geavanceerde toetsenbordinstellingen),
  Wisseltoetsen (`togglekeys-enable`; Sneltoets voor Wisseltoetsen). Schermtoetsenbord en Print Screen:
  Schermtoetsenbord (`screen-keyboard-enabled`), De Print Screen-toets gebruiken om schermopname te openen (Print in
  `show-screenshot-ui`, through `input.shortcuts.set`: a conflict shows "Print Screen wordt al gebruikt door ..."
  and the switch goes back). Meldingsvoorkeuren: Een geluid afspelen wanneer ik Plaktoetsen, Filtertoetsen of
  Wisseltoetsen via het toetsenbord in- of uitschakel (`feature-state-change-beep`). Gerelateerde instellingen: Typen,
  Sneltoetsen.
* **Muis** (`pages/a11y_interaction.rs`): Muistoetsen (expander, switch = `mousekeys-enable`; Snelheid van
  muistoetsen = `mousekeys-max-speed` 10-1000 px/s; Versnelling van muistoetsen = `mousekeys-accel-time`, 3000 ms
  Langzaam to 100 ms Snel); Een venster activeren door de muisaanwijzer erop te plaatsen (`focus-mode` sloppy / click);
  Dubbelkliksnelheid (the mouse's `double-click`, mouse.rs' ten steps, with the time in ms). Klikhulp: Gesimuleerde
  secundaire klik (switch on the header = `secondary-click-enabled`, Acceptatievertraging 0,5-3,0 s) and Klikken door
  te zweven (`dwell-click-enabled`, Vertraging 0,2-3,0 s, Bewegingsdrempel 0-30 px). Gerelateerde instellingen:
  Muisaanwijzer en aanraken (GNOME's Locate Pointer is there), Muisinstellingen.
* Shared (`a11y_vision.rs`, `pub(super)`): `switch_expander` (gs.rs's switch on an expander header, with touchpad.rs'
  `header_switch`, now `pub(super)`), `shortcut_card`, `scale_row` and `value_text` (a fixed-width value text: slice
  3's finding), `write_together` (keys of one schema in one `gsettings.set`, the controls following each key's
  `gsettings.changed`), `keep` (the page holds its `Gs` until it goes, note 11). Control names follow gs.rs; the
  ones without a key of their own: `st-magnifier-view`, `-zoom-in`, `-zoom-out`, `-mag-factor` (the level text),
  `-keep-pointer`, `-follow`, `-mouse-tracking-follow`, `-colour`, `-brightness`, `-contrast`;
  `st-narrator-preferences`; `st-a11y-audio-test`; `st-a11y-keyboard-filterkeys` (+ `-expander`),
  `-enable-sticky`, `-enable-filter`, `-enable-toggle`, `-repeat`, `-show-screenshot-ui`; an expander header is its
  switch's name + `-expander`; a value text its slider's name + `-value`.

### plan.json controls dropped or changed

* Vergrootglas: Windows' Zoomtoename (25-400 %) has no GNOME key ("increment: WinPoint only" in windows.json) and
  settings-defaults.json is frozen: − and + step by GNOME Settings' 25 %. Randen van afbeeldingen en tekst afvlakken,
  Vergrootglas starten na / vóór aanmelden (GNOME's key persists; the login screen is GDM's), the Narrator cursor and
  the Reading section have no GNOME function. Weergave: windows.json maps Vastgezet to the top half; GNOME has four
  halves, so Vastgezet keeps the half Schermgebied chooses (top first). Lens writes lens-mode true and, when the
  position is the full screen, the top half too (GNOME Shell's magnifier ignores a lens over the full screen:
  magnifier.js `_changeROI`); this is the one write beyond a single GNOME Settings control (GNOME Settings writes the
  two keys from two rows). The plan's "De muisaanwijzer houden: ... / Met de inhoud meebewegen" has all three of
  GNOME's ways; Windows' Vergrootglas laten volgen boxes write `focus-tracking` / `caret-tracking` ('none', or the
  schema's default when ticked): GNOME's keys without a GNOME Settings control, named by windows.json.
  Dradenkruis' Lengte: GNOME shows 0-100 % without saying of what; here 0-100 % maps linearly onto the schema's
  20-4096 px. Kleur of the crosshairs has no alpha in WinPoint's colour picker, so the alpha of GNOME's colour button
  is its own row, Dekking (`cross-hairs-opacity`). Windows' Kleurfilters page (system-wide colour filters) has no
  GNOME key (windows.json): it is not built, and Kleuraanpassing works only inside the magnifier, as GNOME's.
* Verteller: Windows' Stem, Uitgebreidheid, Muis en toetsenbord (Vertellertoets) and Braille are Orca's own
  preferences (the one card opens them). Orca 50 has GSettings schemas (`org.gnome.Orca*`, relocatable per profile)
  that a later step could bind for a subset (windows.json: "optional subset"), but they are in no allow table and
  Orca migrates its own settings; left to Orca. Verteller starten na / vóór aanmelden and Volume van andere apps
  verlagen: no GNOME function.
* Audio: Windows' "De titelbalk van het actieve venster laten knipperen" would write the same frame-flash as "Het
  actieve venster laten knipperen" (GNOME has one window flash), so it is left out: a value read back would not tell
  the two apart. Mono-audio has no GNOME function. Testen is GNOME's Test Flash (Windows has no such button).
* Toetsenbord: amendment 24 as the contract says (the `enable` row in each expander, no separate row); the plan's
  separate "Sneltoetsen voor toegankelijkheidsfuncties inschakelen" row is that. Windows' Pictogram voor ... op de
  taakbalk weergeven, Sneltoetsen vergrendelen wanneer ze twee keer achter elkaar worden ingedrukt, Melding
  weergeven ..., the Filtertoetsen timers beyond GNOME's delays and Toegangstoetsen onderstrepen have no GNOME
  function. Filtertoetsen' Tekenherhaling is a link to Geavanceerde toetsenbordinstellingen (slice 2's rows, GNOME's
  Repeat Keys), as the plan says. Print Screen is added (windows.json: "skip the Print toggle if WinPoint has its own
  snipping tool"; it has none). The descriptions of the shortcut rows say what GNOME's AccessX does (Shift five times
  for Sticky Keys, Shift held eight seconds for Slow Keys; Windows says the right Shift).
* Muis: the Muistoetsen sliders are GNOME keys without a GNOME Settings control (windows.json: "the sliders are
  extras"); their ranges (10-1000 px/s, 100-3000 ms) are WinPoint's around GNOME's defaults (10 px/s, 300 ms: GNOME's
  speed default sits at the slow end). Windows' Alleen muistoetsen gebruiken wanneer Num Lock aan is, the taskbar
  icon and Ctrl / Shift for faster / slower have no GNOME function; GNOME's `mousekeys-init-delay` has no Windows row
  and stays as it is. Dubbelkliksnelheid uses the Muis page's ten steps (GNOME's Double-Click Delay has the same ten).

### Notes for I

19. Contract §5.2 (I's CHANGELOG / VALIDATION): batch A's `input.*` now has a method §5.2 does not list,
    `input.screenReader.preferences {}` (amendment 36's "call it from the host"), besides slice 2's
    `input.sources.current` / `.activate`. No bridge method: the host calls Orca directly, so panelBridge.js needs
    nothing for it (§8 note 1 stays as it is).
20. Note 4 (`ToggleSwitch::keep_clicks`) now has eight more users: every switch on an expander header of this slice
    (Vergrootglas, Dradenkruis, Verteller, Plaktoetsen, Filtertoetsen, Wisseltoetsen, Muistoetsen, Gesimuleerde
    secundaire klik, Klikken door te zweven) goes through touchpad.rs' `header_switch` (`pub(super)` now), from
    `a11y_vision.rs` `switch_expander`. With the kit method, `switch_expander` calls `keep_clicks()` instead and
    `header_switch` goes.
21. Note 3 (`Gs::set_values`): `a11y_vision.rs` `write_together` is the same workaround for five controls (Weergave,
    Helderheid, Contrast, Mijn scherm laten knipperen, the Filtertoetsen switch): they write through `p.run_with`
    and follow `gsettings.changed`, not optimistic. With `Gs::set_values` they become optimistic.
22. Amendment 24's wording: the contract says the one `enable` row "covers all three". In mutter's AccessX the
    keyboard toggles are Shift pressed five times (Sticky Keys) and Shift held eight seconds (Slow Keys); I found no
    keyboard toggle for Toggle Keys (GNOME's Sound Keys). The page follows the contract (the row in all three
    expanders, "geldt voor Plaktoetsen, Filtertoetsen en Wisseltoetsen samen"); the Wisseltoetsen row's description
    says only that it is the same setting. If I wants it exact, the Wisseltoetsen expander could drop the row.
23. Optional, registry (frozen): `a11y-mouse`'s keywords could add "venster activeren" / "activate window" and
    "muistoetsen snelheid"; `magnifier`'s "dradenkruis" / "crosshairs", "kleuren omkeren" / "invert colours";
    `narrator`'s "orca" is there.
24. Harness, for every batch: `choose()`-style helpers that pick a label by text must skip the kit switches' state
    texts (`w11-toggle-state`: "Aan"/"Uit", "On"/"Off"); the first slice-4 run chose Overversterking's "Uit" instead
    of the drop-down's option Uit. And GLib's keyfile backend writes an empty `as` as `[]`, not `@as []`.

### Tests

* Host (`services::input::a11y_tests`, 3 new): every key the five pages bind has its allow row in the mode its
  control needs (Write; media-keys and `show-screenshot-ui` `Service("input.shortcuts.set")`; `audible-bell` in
  none); the writes the pages send pass `gsettings.set`'s checks and read back GNOME's values on a memory backend
  (lens-mode with screen-position, the RGB brightness and contrast triples, mag-factor, the tracking enums, the
  crosshairs' length, colour and opacity, the visual bell pair, Filtertoetsen off, the delays, the mouse keys
  sliders, Klikhulp's seconds and pixels, focus-mode), a dry run writes nothing, GNOME's ranges hold (a crosshair
  below 20 px, a brightness above 1: refused, and a refused three-key write writes none of them);
  `input.screenReader.preferences` against an Orca fake on the sandbox's private session bus: nobody there
  (`unsupported input.orcaNotRunning`, also for the well-known name before the fake owns it), `{shown: true}` by
  the unique and by Orca's well-known name, false → `failed input.orcaFailed`; every call recorded as a plain
  `ShowPreferences` without the interactive flag and with `NO_AUTO_START`.
* Native: `pages::a11y_vision::tests` (5 new: the zoom quarters and 100-2000 %, the view of lens-mode and
  screen-position and its round trip, GNOME's follow behaviour and the tracking defaults, the crosshairs' length and
  colour, every magnifier key's control name of its own), `pages::a11y_interaction::tests` (4: the visual bell pair
  and its round trip, Filtertoetsen = Slow or Bounce Keys, Print in show-screenshot-ui keeping its other keys, the
  mouse rows' mappings and the control names of both pages).
* Harness: `tests/gnome/scenarios/settings-a11y.py` parts 7-11 (module docs), with an Orca stand-in
  (`tests/gnome/lib/standins/a.py` `SPECS['orca']`: org.gnome.Orca.Service on the private session bus). What they
  check by real clicks: the switches on the expander headers of Vergrootglas, Dradenkruis, Verteller, Plaktoetsen,
  Filtertoetsen, Muistoetsen and Gesimuleerde secundaire klik write their keys; + writes mag-factor 2.25 and shows
  225%, mag-factor 3.0 from outside shows 300%; Weergave Lens writes lens-mode true with screen-position top-half,
  Volledig scherm false with full-screen; Instellingen voor Verteller openen is insensitive while the screen reader
  is off and then calls ShowPreferences on the stand-in exactly once, without interactive authorization; the screen
  flash writes visual-bell with fullscreen-flash and frame-flash, Uit visual-bell false; amendment 14's note; the
  shared Sneltoets row follows in Filtertoetsen (amendment 24); Filtertoetsen off writes both keys false; Print
  Screen off and on writes [] and ['Print']; focus-mode sloppy and click. The magnifier is switched off from outside
  right after its click, so no later click lands in a zoomed screen. The first run found two scenario faults (both
  in the scenario, note 24), no page fault; the screenshots led to the notes' alignment and the missing glyphs
  (commit adb47447).
* Results of this slice: host `--lib` 612 passed, 1 failed, 19 ignored (the failure is B0's
  `elf::our_binaries_equal_readelf`, libm.so.6, note 5; slice 3 had 609 passing); native `cargo test --release` 104
  passed, 1 ignored (slice 3: 95); settings-a11y.py nl 93/93 and en 104/104 checks (no Dutch word in English, no
  pkexec, tecla or orca call in guard-calls.log: only the shell's wpctl and pw-dump); builds: host warnings 31 and
  native warnings 20, both equal to base; checks.sh as at base (rustfmt, no-webkit, i18n, pairs, rust 57 warnings,
  drift 15, native 48 uncovered, test-native, settings-pages and release pass; installer rc 70: this shell's
  XDG_RUNTIME_DIR). All 109 new UI strings are in `tests/i18n/windows-glossary/a.tsv`, verified=no.

### Batch A after slice 4

All 18 pages of §4.2 are built; no page of batch A uses `common::placeholder` any more. Open notes for I: 2, 3, 4, 5, 6,
7, 8, 9, 10, 11, 13, 14, 15, 16 and 17 (slices 1-3), 18 (pen.rs' caps refresh, A's own file, left as it is: a
second tablet plugged in while one is there changes no caps value, so a narrower refresh would miss it), and 19-24
above.

## Harness step: settings-input.py, settings-a11y.py and the stand-ins

Contract §4.2 gives batch A `tests/gnome/scenarios/settings-input.py` and `settings-a11y.py`, and §3 gives it
`tests/gnome/lib/standins/a.py`. This step brings both scenarios up to plan.json's GNOME HARNESS list for all 18
pages. Each runs in nl and en; settings-input.py runs with `--strace`, because Taalopties reaches pkexec.

### What the scenarios check now

* **Every page.** It is opened as Start's search opens it (`h.open('settings:<id>')`) and screenshotted
  (`<id>.<lang>`, plus a few state shots). The title and breadcrumb are checked against the registry's parent
  chain in the run's language, each earlier crumb by its `crumb-<id>` name. In en there must be no Dutch word.
  The keys the page binds are checked through `gsettings.get`: none is Owned, so no page shows a pointer to a
  WinPoint page instead of a control (batch A binds no Owned key). The Mirrored ones name their setting:
  high-contrast is `accessibility.highContrast`, reduced-motion and enable-animations are
  `accessibility.reduceMotion`, text-scaling-factor is `accessibility.textScale`.
* **Every bound switch and check box** is flipped by a real pointer click (`flip`). Before the click the page is
  scrolled there by real wheel steps over its left margin (`reveal`), and its expander is opened first. After the
  click:
  * the keyfile holds GNOME's exact GVariant text: the negated bool, or the mapped enum value (accel-profile
    'flat'/'default', click-method 'areas'/'fingers', focus-tracking 'proportional'/'none', caret-tracking
    'centered'/'none', reduced-motion 'reduce', focus-mode 'sloppy', overlay-key '', send-events
    'disabled-on-external-mouse');
  * the control shows the new value;
  * the old value, written from outside with `h.gsettings_set`, moves the control back (`gsettings.changed`).

  Controls that are sensitive only while another key is on (tap-and-drag, the Slow Keys beeps, the Bounce Keys beep)
  get that key written from outside first, and it is restored afterwards. Klikken door te zweven gets dwell-time
  3.0 s while it is on, so it cannot click by itself before the outside write turns it off again.

  Each page ends with `unflipped`: every visible `st-<page>-*` GtkSwitch and GtkCheckButton was flipped, or is
  skipped with its reason (the fixed Inactieve vensters schuiven, the fixed two-finger tap and pinch boxes). In every
  run, every page's list was empty.
* **Sliders and drop-downs.** Every slider not yet covered gets one real click, and its key must be the page's
  mapping of that position within a step or two:
  * Cursorsnelheid, Vertraging voor herhalen, Herhalingssnelheid;
  * Helderheid, which writes its three keys together; Kleur, Dikte and Dekking;
  * the Slow and Bounce Keys delays, both mouse-keys sliders, Dubbelkliksnelheid, the secondary-click delay, and the
    dwell delay and threshold.

  Where a value text exists, an outside write must change it. Outside writes must also move these drop-downs:
  Primaire muisknop, Het aanraaktoetsenbord weergeven, Hoog contrast and the screen flash.
* **caps and the search.** caps comes from `caps.get`. The real search (text, then Enter) finds:
  * nl "muis" / en "mouse": Muis first, before Toegankelijkheid > Muis (amendment 65);
  * Touchpad exactly while caps.touchpad, and Pen en tablet exactly while caps.tablet (both false on this machine,
    so both are hidden);
  * Sneltoetsen, and the Vergrootglas and Verteller links, exactly while caps.gnome (true).

  Opened directly, Touchpad shows its "no touchpad" notice and Pen en tablet shows "Geen tablet gevonden".
* **polkit** (Taalopties, the only system write of batch A), with two stand-ins of `org.freedesktop.PolicyKit1` on
  the private system bus (`SPECS['polkit-challenge']`, `SPECS['polkit']`):
  * A keyboard added while polkit would ask: CheckAuthorization of `org.gnome.controlcenter.set-keyboard` reaches the
    stand-in once. Its subject is `unix-process` (the backend's pid, start time and uid), with flags `uint32 0` and
    no ALLOW_INTERACTIVE_AUTHORIZATION, as GNOME Settings' GPermission check makes it. No pkexec runs.
  * With polkit saying yes: the run's pkexec stub logs `/usr/libexec/gnome-control-center-set-debian-keyboard
    nl,us ,intl`, and `us,nl intl,` after Omhoog.
  * The strace exec check shows only the stub's pkexec ran (`run/stubs/pkexec`, 2 runs), and the connect check is
    clean.

  A has no polkit call with interactive=true: the contract (§5.2, §5.8) makes this one a no-prompt check, so the
  scenario requires interactive=false. Orca's ShowPreferences is a plain call without the flag, sent once, and
  WinPoint starts no `orca` program.
* **Kept from the slices:** the mouse reference case (left-handed true, speed 0.5 from slider position 15,
  accel-profile 'flat', natural-scroll true), the text scale applied once, the mirror's pairs, high contrast in the
  window and the shell, the quick settings tile, Orca, amendments 13, 14 and 24, Super+Q by real keys, the custom
  shortcut and Alles opnieuw instellen.
* **Navigation:** a real click on Geavanceerde toetsenbordinstellingen' crumb Typen goes there, and a Vergrootglas
  shortcut row opens Sneltoetsen with Toegankelijkheid open.

### What the runs found, and the fixes

1. **Sneltoetsen did not follow a change made elsewhere** (page fix, `native/apps/settings/src/pages/shortcuts.rs`).
   The page rebuilt itself only on this host's own `input.changed`, so a shortcut written by another program stayed
   stale; GNOME Settings' rows follow their keys. Now:
   * the page asks `gsettings.get` for one key of every schema and path it shows (`watched_keys`: wm, media-keys,
     shell, mutter keybindings, the custom list and each custom path), which opens them in the host's gsettings
     service;
   * it rebuilds on their `gsettings.changed` (`follow_elsewhere`);
   * the host's own `input.changed` and the `gsettings.changed` of the same write are merged into one rebuild 300 ms
     later (`soon`).

   There is a new native test, `the_page_watches_one_key_of_every_schema_and_path_it_shows`. The harness writes
   close ['<Alt>F4'] from outside and requires the row to change from Windows Q to Alt F4.
2. **The tool seam was refused in every harness run started from this shell** (harness issue; note 26 below). No
   polkit call was made and no pkexec stub could log, because `host/src/tools.rs` refuses a group-writable
   `WIN11_SYSTEM_TOOL_DIR`, and the harness creates `run/stubs` with the process umask (0002 here, so 0775).
   `settings-input.py` and `settings-a11y.py` call `private_stubs()` first: chmod 0755 inside `tests/gnome/run`,
   then a check. After that the polkit and pkexec checks above pass.
3. **Real clicks on a kit Dialog's buttons and check boxes do nothing** (kit bug; note 25 below, which corrects note
   7(b)). Confirmed in every run: a real click on Eigenschappen voor Muis' Sluiten leaves the dialog open (the check
   carries `known='kit-dialog-claims-presses'`), while a real click on the dialog's Dubbelkliksnelheid slider does
   write 700 ms. Inside dialogs the scenarios use the dev channel's click.
4. Scenario faults, fixed in the scenarios:
   * the Sneltoetsen row was read before the 300 ms rebuild after Super+Q;
   * a collapsed expander's rows kept a stale rectangle, so `reveal` now opens a named expander before anything else.

The screenshots show no layout problem against Windows 11 (cards, expanders, rows with the control on the right,
breadcrumbs, the search results with their parent paths, the dialogs).

### Notes for I

25. **`native/kit/src/controls/dialog.rs` (kit, not A's): with a real mouse or touch, no button or check box inside a
    kit `Dialog` can be clicked.** The dialog layer (`smoke`) has a `GtkGestureClick` in the bubble phase that claims
    every press ("Clicks on the smoke stay there"). The dialog is a child of the smoke, and GTK 4's GtkGesture docs
    say a claim emits `cancel` on every gesture of the widgets underneath in the propagation chain. So the buttons'
    and check boxes' own click gestures are cancelled before their release: no `clicked`, no toggle. Sliders still
    move, because GtkRange acts on the press itself.

    This hits every WinPoint dialog, not only batch A's: Eigenschappen voor Muis, the keyboard and custom-shortcut
    dialogs, `c::confirm`, the other batches' dialogs and the other apps. Fix: claim only presses on the smoke
    itself, so modality stays:
    ```rust
    let block = gtk::GestureClick::new();
    block.connect_pressed(|g, _, x, y| {
        if let Some(w) = g.widget()
            && w.pick(x, y, gtk::PickFlags::DEFAULT).as_ref() == Some(&w)
        {
            g.set_state(gtk::EventSequenceState::Claimed);
        }
    });
    ```
    After it, `dialog_click_check` in settings-input.py should pass, and its `known` mark (and the dev clicks inside
    dialogs) can go.
26. **`tests/gnome/harness.py` (frozen): `run/stubs` must be private.** It is made with `mkdir(parents=True)` under
    the process umask; under umask 0002 it is 0775, and `tools.rs` then refuses every `system_tool` spawn. So pkexec,
    tecla, grdctl, passwd, nmcli and the rest are never run, and their stubs never log. Every batch whose scenario
    checks a helper's argv is affected (C's grdctl, E's passwd and localedef, ...). Patch: after the stubs are
    written, `(RUN / 'stubs').chmod(0o755)` (or `os.umask(0o022)` at the start of `launch`). Then `private_stubs()`
    in batch A's scenarios only checks.
27. Harness helpers for the other batches and `settings-tour.py`:
    * `h.click` falls back to the dev click for a widget scrolled outside the window, and makes a real click for one
      scrolled under Settings' fixed header, which lands on the header.
    * settings-input.py's `reveal` (real wheel steps until the widget's centre is inside the content's
      GtkScrolledWindow), `flip`, `slide` and `unflipped` would fit `tests/gnome/lib/scenario.py` (frozen) as shared
      helpers.
28. **caps (frozen) has no fixture seam,** so the harness cannot show the positive side of a hardware page: caps
    reads the host's read-only `/run/udev/data` and `/sys`. A `WIN11_CAPS_ROOT` for `scan_files`' root (stubs only,
    like `WIN11_SYSTEM_TOOL_DIR`) would let settings-input.py add a fixture udev entry with
    `ID_INPUT_TOUCHPAD=1` / `ID_INPUT_TABLET=1` and see Touchpad and Pen en tablet appear in the search and the
    navigation. Until then, the positive side is the native `caps_tests` and the host's caps fixture tests.
29. The Bluetooth en apparaten page (B's `bluetooth.rs`) lists no Muis, Touchpad or Pen en tablet in this worktree,
    while Windows lists them under Bluetooth en apparaten. If B does not add them, I could add
    `common::child_links(&p, "bluetooth")` there: it is caps-filtered, so Touchpad and Pen are hidden without the
    hardware. Muis is reachable now from the search, Start and the Toegankelijkheid > Muis related link.

### Tests of this step

* Final runs (`tests/gnome/scenarios/settings-*.out/results.json`, logs committed, PNGs local and gitignored):

  | Run | Checks passed | Note |
  |-----|---------------|------|
  | settings-input.py, nl, `--strace` | 161 of 162 | the one failure is the known kit Dialog check (note 25) |
  | settings-input.py, en, `--strace` | 168 of 169 | the same known check |
  | settings-a11y.py, nl | 249 of 249 | |
  | settings-a11y.py, en | 260 of 260 | |

  * In en there are no Dutch words: 10 page checks in settings-input.py and 11 in settings-a11y.py.
  * Real pointer input: 52 clicks in settings-input.py and 97 in settings-a11y.py. The dev channel clicked 8 times in
    settings-input.py (7 of them inside kit Dialogs) and twice in settings-a11y.py (h.click's own fallback).
  * The input runs' strace exec check is OK: only `run/stubs/pkexec` ran, twice. The connect check is OK too.
  * guard-calls.log lists only the shell's wpctl and pw-dump, plus the two pkexec stub calls of Taalopties. No tecla
    or orca ran.
* Earlier runs of this step (logs not kept):
  * input nl 153 of 160: the seam and the Sneltoetsen follow, before the fixes above;
  * a11y nl 248 of 249: the collapsed-expander rectangle;
  * input nl 160 of 161 and en 167 of 168: before the touchpad alignment fix.
* Regression in nl, on this step's build:
  * `native-shell.py` 21 of 21 and `lock-unlock.py` 5 of 5, the same as their committed results;
  * those result dirs belong to other steps, so their files stay at HEAD (the run's copies are in this session's
    scratchpad).
* Native `cargo test --release`: 105 passed, 1 ignored (slice 4 had 104; the new test is Sneltoetsen'
  `watched_keys`). Native build warnings: 20, as at base.
* `checks.sh a`: as at base. rustfmt, no-webkit, i18n, pairs, rust (57 warnings), drift (15), native (48
  uncovered), test-native, settings-pages and release pass; installer rc 70 (this shell's XDG_RUNTIME_DIR).
* No new UI string: the two page fixes add none.

### Batch A after the harness step

Open notes for I: 2-11, 13-17, 18 (pen.rs' caps refresh), 19-24 (slice 4) and 25-29 above. Note 25, the kit Dialog,
is the one users meet: no real click on a button or check box in any WinPoint dialog works until it is applied.


## Codex continuation, 17-11-2026: review repairs, not built or runtime-tested

Starting point: ac98ef65. Robin explicitly requested review without builds or interference with the live session,
then asked to continue. The review identified five repairs (A-R1 through A-R5); this continuation changes only
A-owned source files and this note. No installed component or live setting was touched; nothing was pushed.

* A-R1, shortcuts.rs: removing a custom shortcut resets enable-in-lockscreen when supported by the installed
  schema. Adding one explicitly sets it false, including an orphaned customN path left by an older application.
* A-R2, shortcuts.rs: replacement plans include every conflicting owner, the target values and (for custom.add)
  the parent custom-keybindings list. Every step is validated in dry-run mode before the first write. A target
  held by WinPoint is preflighted with the existing held check deferred only when an earlier validated step frees
  the matching extension binding. Runtime writes revalidate. On a later failure the operation attempts restoration
  in reverse order, preserving each key's default status. GSettings has no cross-schema transaction: policy changes
  can block rollback too; such a failure is exposed in the API error data as rollbackFailed=true.
* A-R3, shortcuts.rs: one owner-removal operation subtracts all requested accelerators, not just the first conflict
  reported for that owner, and preserves unrelated accelerators.
* A-R4, pen.rs: await the reset before opening calibration, skip the window in Test mode, disable the launch button
  while pending, restore the original area if navigation completed during reset, and await final save before the
  success toast. A failed final save attempts to restore the pre-calibration area. Cancel/close restores it too.
* A-R5, accessibility.rs: Text size awaits the host response, suppresses success in Test mode or on refusal, and
  disables repeated Apply requests while the save is pending.

Four regression tests have been added to input::shortcuts::replacement_tests, but NOT run:

1. Remove two requested accelerators from the same owner while retaining a third.
2. Reject an invalid later write without changing the original shortcut or its default status.
3. Reset lock-screen permission on removal and explicitly clear it when reusing an orphaned custom path.
4. A keyfile backend with the custom-keybindings list locked: replacement must not steal the lock-screen shortcut.

Light validation after the edits: tools/rustfmt-check.sh (190 host files, 0 failures, 30 existing exceptions),
check_pairs.py (0 errors/0 warnings), check_native.py (0 errors, the same 48 native-dutch-only warnings), parsing the
two edited page files with rustfmt --emit stdout, and git diff --check. No build, cargo test, D-Bus fake, GNOME
harness, install, session action or push was performed. These checks do not type-check the new Rust or establish
runtime correctness. The work must remain WIP until the allowed sandbox build/test/review loop is completed.

Next validation when builds and isolated tests are allowed: existing input/keyboard tests (especially replacing
WinPoint's held Alt+Tab), the four new replacement tests, settings-input.py and settings-a11y.py in nl/en, plus
injected delayed/refused calibration and Text size writes and Test mode. The existing K1/K2/K3, H4/DF19 and other
integration notes above remain open. Do not merge A as finished on this source-only evidence.


### Follow-up to host-settings H-R1, 17-11-2026 (source only)

The shared reset guard in package f is also applied to A's current local reset_keys: validate effective defaults
against the extension's held accelerators before touching any key, including dry-run. resetAll skips only
heldByWinPoint refusals and propagates other failures, so an all-reset does not steal Alt+Tab/Ctrl+Alt+Delete or
abort just because WinPoint holds one default. A new memory-backend regression covers single-key refusal, dry-run,
all-reset preserving the held key and resetting an unrelated key. The Rust test has not been built or executed.
At DF19 keep resetAll's skip logic when substituting gsettings::reset_values_in for the local helper.
No build, installed-file change, live-session call or push was performed.
