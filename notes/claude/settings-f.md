# Settings parity, batch F: handoff note

Contract: `notes/claude/settings-parity.md` (§1 amendments 4-7, 22, 38, 42, 43, 45, 63; §2; §4.7; §5.7). Batch F runs in
three slices on branch `claude/settings-f`; each slice appends its section here.

## Slice 1: Apps, Geïnstalleerde apps, Geavanceerde opties, Standaard-apps

### Host

`host/src/services/privacy.rs` + `privacy/{perms,flatpak,snapd,shortcuts,handlers,app,tests}.rs`, fragment
`host/src/api-methods/apps.json`, stem rows `host/i18n/privacy.tsv` (`#@ new`, listed in
`tests/i18n/BASELINE-CHANGES.rust-settings-f`), `host/src/apps/defaults.rs`.

| method | dangerous | what |
|---|---|---|
| `privacy.permissions {table, id}` | no | PermissionStore `Lookup`: `{available, apps: [{app, value, allowed}]}`; a missing entry is empty, no store on the bus is `available: false` |
| `privacy.setPermission {table, id, app, value}` | yes | `SetPermission`; `value` true/false becomes GNOME's own values (yes/no, EXACT/NONE with the kept timestamp, GRANTED/DENIED) or the strings themselves, checked per kind |
| `privacy.deletePermission {table, id, app?}` | yes | `DeletePermission`, or `Delete` of the whole entry without `app` |
| `privacy.app {id, sizes?}` | no | GNOME's per-app page in one answer (see the fragment's doc); `sizes: false` skips the disk walk (Installed apps' menu uses it) |
| `privacy.app.clearCache {id}` | yes | a Flatpak's `~/.var/app/<id>/cache` emptied (entries only, never through a symlink); `{freed}` |
| `privacy.app.resetHandlers {id}` / `{all: true}` | yes | `g_app_info_reset_type_associations` for the app's types, or for every type the user's mimeapps.list names ("Alle standaard-apps opnieuw instellen") |
| `privacy.app.removeHandler {id, type}` | yes | `g_app_info_remove_supports_type` (only a type the app's `MimeType=` lists) |
| `privacy.app.snapConnect {id, plug, slot?, connect}` | yes | snapd `POST /v2/interfaces` with `X-Allow-Interaction: true` (amendment 42), then `/v2/changes/<id>` until Done; without `slot`: the connected one (disconnect) or the only candidate (connect) |
| `privacy.app.globalShortcuts.set {id, shortcuts?, clear?}` | yes | `RebindShortcuts(s, a(sa{sv}))`: every registered entry, the named ones with their new accelerators, all without any for `clear` |
| `privacy.app.setAutostart {id, on}` | yes | **contract addition**, see the notes for I |

Kinds of the PermissionStore (`perms::KINDS`): location/location, devices/camera, devices/microphone,
screenshot/screenshot, wallpaper/wallpaper, gnome/shortcuts-inhibitor (amendment 7), background/background; nothing
else is reachable. Events: `privacy.changed {what: "permissions" | "app" | "handlers", ...}` after every real write
(none in a dry run), and `privacy.changed {what: "permissions", table, id}` for every PermissionStore `Changed` signal
of those entries while the service lives (review fix 6, below).

`apps.setDefault` takes `calendar` (text/calendar), `calls` (x-scheme-handler/tel) and `sms` (x-scheme-handler/sms);
`apps.defaults` reports them and their candidates (`SPECS` order: browser, mail, calendar, calls, sms, files, music,
video, image, text, terminal).

Allow rows: none new. The pages read and write org.gnome.desktop.search-providers (`*`, Write), privacy
`disable-camera`/`disable-microphone` and location `enabled` (B0's rows in `allow/apps.rs`). The global-shortcuts
schemas are read natively in `privacy/shortcuts.rs` (amendment 43), in no allow table.

Tests (`host/src/services/privacy/tests.rs`, `apps/defaults.rs`): PermissionStore values and lookups, Flatpak
metadata → GNOME's static rows, AppStream version, the disk walk (hard links once, the skipped cache, symlinks never
followed or emptied), snapd answers (Content-Length and chunked bodies, errors with their kind → core.notAuthorized
/ core.authCancelled / privacy.snapUnavailable), a snap's plugs and slots, global-shortcut variants and the rebind
call, globs2 extensions, mimeapps.list types, autostart files, search-provider .ini files, bad arguments in nl and
en. Against fakes: a PermissionStore on the sandbox's private session bus (dry run writes nothing, the values
stored, the calls never ALLOW_INTERACTIVE_AUTHORIZATION, the event), the GlobalShortcutsRebind (the rebind
arguments, the dry run), a snapd on a socket of its own (reads without and the POST with X-Allow-Interaction, the
change waited for, the slot rules, no POST in a dry run, snapd gone), and in the sandbox the handlers, Autostart and
Clear Cache writes on a test app. The fake tests are in the normal `--lib` run (B0's `gnome.rs` pattern: an object
on its own connection to the private bus), not in tests/dbus-fakes, so `tests/dbus-fakes/fakes/f/` stays empty.

### Native

* `pages/apps.rs`: unchanged (the registry's child links).
* `pages/installed.rs`: the "…" menu gains Geavanceerde opties (→ `app-advanced {id}`, not for WinPoint's own
  apps) and Openen in {store} (GNOME's App Details: App Center with `snap://<name>` for a snap, GNOME Software with
  `appstream://<id>` otherwise, through `apps.launch` with the link; resolved per app when the menu opens, so it is
  greyed for an app no installed store can show, review fix 7); Sorteren op
  gains Installatiedatum; an empty list offers Zoeken in {store} (amendment 7). `software` is `pub(super)` for
  app_advanced.rs's Verwijderen.
* `pages/app_advanced.rs` (data page `{id: desktop id}`, title = the app's name; the section order is Windows' since
  review fix 10, see there): header (icon, kind, version, Openen, Openen in {store}); Specificaties (Versie, App,
  Gegevens, Cache, Totaal); Opnieuw instellen (Cache wissen with a question, between Beëindigen and Verwijderen);
  App-machtigingen (the PermissionStore rows the app asked for, as GNOME: a row only when the entry has the app;
  "Uitgeschakeld voor alle apps" while privacy disable-camera/-microphone or location enabled says so; Meldingen →
  `app-notifications {id}`; Zoeken, bound to search-providers `disabled` or `enabled` by DefaultDisabled, insensitive
  with `disable-external`); Machtigingen voor apps op de achtergrond (sandboxed apps: Altijd / Nooit); Wordt
  uitgevoerd bij aanmelden (insensitive while the background permission is Nooit, amendment 7); Algemene
  sneltoetsen (a dialog: Wijzigen through `pickers::KeyCapture`, Alles verwijderen with GNOME's question);
  Vereiste machtigingen (Flatpak: GNOME's sentence and its static rows; snap: a switch per plug, a slot drop-down
  when there is more than one); Standaardkoppelingen (`handlers_group`, shared with defaultapps.rs: Bestandstypen /
  Koppelingstypen, Standaard or Als standaard instellen, Type verwijderen, Opnieuw instellen); Beëindigen (the app's
  group in sysmon.processes → sysmon.endTask); Verwijderen (software.uninstallInfo, then installed.rs's reviewed
  plan). Control names `st-app-advanced-<key>` (`-camera`, `-background`, `-autostart`, `-disabled`/`-enabled` for
  Zoeken via `gs::control_name`, `-plug-<plug>`, ...).
* `pages/defaultapps.rs` (amendment 22): Een standaardwaarde instellen voor een bestandstype of koppelingstype (a
  search over apps.mimeTypes, 20 hits; a row's button opens "Een standaard-app selecteren voor {type}" with the apps
  of apps.forType, then apps.setDefaultForType), Standaardwaarden instellen voor toepassingen (the apps, searchable;
  an app opens `defaultapps {id}`: its Files & Links through `app_advanced::handlers_group` and a link to its
  Geavanceerde opties), the WinPoint addition "Standaard-apps per soort" (an expander with GNOME's Default Apps rows,
  candidates now from apps.defaults `{candidates: true}`: Webbrowser, E-mail, Agenda, Muziekspeler, Videospeler,
  Fotoviewer, Oproepen and Sms only when an app handles them, then Bestandsbeheer, Teksteditor, Terminal), and
  Gerelateerde instellingen: Standaardwaarden kiezen op bestandstype / koppelingstype (`defaultapps {view}`, 100 types
  at a time, Meer weergeven) and Alle standaard-apps opnieuw instellen (a question, privacy.app.resetHandlers {all}).
* UI strings: `tests/i18n/windows-glossary/f.tsv` (128 rows, source windows or winpoint, verified no; glossary rows
  yes). "App Center" is a product name: it is the `{store}` placeholder's value, so no en text carries the US
  spelling.
* Names: the app pages show the app's name as apps.list gives it (WinPoint's language, the catalogue's
  translations); privacy.app's own `name` is GIO's, in the host process's locale. File types are named from
  shared-mime-info's `<comment xml:lang>` in WinPoint's language (`handlers::label`, as apps.mimeTypes does).

### Harness

`tests/gnome/scenarios/settings-apps.py` (nl: `settings-apps.out`, en: `settings-apps.en.out`): Installed apps and
the "…" menu, the Nautilus page reached through Geavanceerde opties with its name as the last crumb, Zoeken flipped
by a real click into search-providers `disabled` in the private keyfile and following a write from outside,
Standaard-apps' order (no drop-down above Windows' two searches), the type search, the kinds, the related views,
an app's view, and no Dutch words in English. Later slices extend it (Meldingen) or add settings-privacy.py.

### Runs (16-11, worktree f, base af8eebdc)

* `wt/host-tests.sh f --lib`: 587 passed, 1 failed, 19 ignored. The failure is note 7 below
  (`our_binaries_equal_readelf` against a recording that predates B0's libm use; before the fresh build the same
  run gave 588 passed, 0 failed). privacy: 13 tests, apps/defaults: 1 new.
* `wt/native-tests.sh f`: 63 passed, 0 failed, 1 ignored (win11-gtk-settings 28, of them 4 new).
* `wt/build.sh f all`: host and native release builds ok, native-release refreshed (19 binaries).
* `wt/checks.sh f`: as at the base (rustfmt 0 failing; check_rust 0 errors, 57 warnings; drift 0 errors, 15
  warnings; native 0 errors, 48 uncovered; test_native OK; release OK). `installer` exits 70 here as at the base
  (XDG_RUNTIME_DIR is not 0700 in this environment).
* Harness `settings-apps.py`: nl 21 checks, en 25 checks, none failed.

### plan.json controls dropped or changed

* installed, "⋯ Openen in App Center": also GNOME Software for Flatpak and system apps (GNOME's own rule off Ubuntu);
  Windows' Wijzigen and Verplaatsen are left out (no GNOME function).
* app-advanced, Achtergrond: two options (Altijd, Nooit); Windows' "Energie-geoptimaliseerd (aanbevolen)" has no
  GNOME value. Unset counts as allowed, as GNOME's `get_flatpak_permissions`.
* app-advanced, App-machtigingen: a row only for a permission the app's entry has (GNOME); Achtergrond is its own
  section (Windows).
* app-advanced, Meldingen (met geluid): a link to the app's `app-notifications` page (slice 2 builds it; it gets
  `{id: desktop id}`), not a switch here.
* app-advanced, Wordt uitgevoerd bij aanmelden: through the new `privacy.app.setAutostart`, not `sysmon.setStartup`
  (that one only flips an autostart file that exists; GNOME's Autostart creates the copy).
* app-advanced, Beëindigen: kept (Windows) although GNOME has no terminate; Windows' Herstellen is left out, its
  Opnieuw instellen is GNOME's Cache wissen, in a group "Opnieuw instellen" of its own between Beëindigen and
  Verwijderen (review fix 10; only for an app whose cache the host can empty). The GNOME additions (Algemene
  sneltoetsen, Vereiste machtigingen, Standaardkoppelingen) come after Verwijderen.
* app-advanced, Specificaties: a version and sizes for Flatpak (deploy, AppStream file, `~/.var/app`) and snap
  (snapd, `~/snap/<name>`) only, as GNOME's Storage section; a system package shows "Er zijn geen specificaties van
  deze app bekend." (no dpkg query in this slice).
* defaultapps, the GNOME drop-downs: below Windows' two lists in an expander, not at the top (amendment 22).

### Notes for I

1. **Contract correction (amendment 43, §5.7):** `org.gnome.GlobalShortcutsRebind` is the *interface*; the bus name
   is xdg-desktop-portal-gnome's `org.freedesktop.impl.portal.desktop.gnome` (gnome-control-center 50's proxy sets
   g-name org.freedesktop.impl.portal.desktop.gnome, g-interface-name org.gnome.GlobalShortcutsRebind, g-object-path
   /org/gnome/globalshortcuts; no .service file names org.gnome.GlobalShortcutsRebind). `privacy/shortcuts.rs` uses
   that. The portal answers "Session for app %s not found in RebindShortcuts" when the app has no GlobalShortcuts
   session (it is not running): the page's error says "Open de app en probeer het opnieuw".
2. **Contract addition (§5.7):** `privacy.app.setAutostart! {id, on}` (apps.json, METHODS); optional arguments
   `privacy.app {sizes}` and `privacy.app.resetHandlers {all}`.
3. **Registry note (`registry.rs`, frozen):** an app's view of Standaard-apps is `defaultapps {id}` with the header
   "Apps › Standaard-apps" (the app's name is in the page's hero). Windows shows "Apps › Standaard-apps › <app>". If
   I wants that crumb: a data page after `defaultapps`,
   `page!("defaultapps-app", Some("defaultapps"), "open", "App", "App", "", "", [], [])` in `PAGES`, `"defaultapps-app"`
   in `DATA_PAGES`; then F's `defaultapps.rs` `PAGES` gets `("defaultapps-app", app_page)` calling `p.set_title(name)`
   (a two-line change in an F file, which I may make).
4. **mimeapps.list lock:** `apps/defaults.rs`'s `ConfigLock` serialises WinPoint's own read-modify-writes of
   mimeapps.list, but `apps/mod.rs` (frozen) declares `mod defaults` private, so `privacy.app.removeHandler` and
   `resetHandlers` write through GIO without it. If I wants them under the lock: `pub(crate) use defaults::ConfigLock;`
   in `apps/mod.rs` and `let _lock = crate::apps::ConfigLock::acquire(&glib::user_config_dir())` around the two GIO
   calls in `services/privacy.rs`.
5. **Sandbox on NTFS:** `ConfigLock` needs a private file (mode 0600, one link); the rust-sandbox's config dir on this
   NTFS worktree cannot have one, so `apps.setDefault`'s write is skipped there by
   `calendar_calls_and_sms_are_kinds_with_their_one_type` (it still checks the candidates through GIO). GIO takes an
   app's MIME types from `mimeinfo.cache`; the tests give their test apps types through Added Associations instead.
6. The tests in `services::privacy::tests` that use the private bus, GSettings or HOME skip outside
   tests/rust-sandbox: `services::privacy::tests::sandbox()` (since review fix 2 the whole environment of
   run-tests.sh: keyfile GSettings, a private session bus, and HOME, XDG_DATA_HOME, XDG_CONFIG_HOME and
   WIN11_SYSTEM_TOOL_DIR inside `$SANDBOX`). apps/defaults.rs, background.rs and notifications.rs use the same guard.
7. **Not F's, seen here:** after a fresh release build every host binary needs `libm.so.6` (GLIBC_2.29 `pow`, from
   B0's `settings_mirror.rs` `powf(2.4)`; the fresh builds of worktrees a-e show the same), so
   `services::update::elf::tests::our_binaries_equal_readelf` fails against the recording
   `tests/update/fixtures/compat/readelf-needed.txt` (not F's file). I: record it again
   (`tests/update/test_compat.sh --record-readelf`): `libm.so.6` between `libgcc_s.so.1` and `libc.so.6` for the
   host binaries.

## Slice 2: Meldingen, Meldingen van app, Persoonlijke instellingen, Achtergrond, Kleuren

### Host

| method | dangerous | what |
|---|---|---|
| `notifications.apps {}` | no | GNOME Settings' App Notifications: the apps whose desktop file says `X-GNOME-UsesNotifications=true`, then those GNOME Shell recorded in `application-children` whose `application-id` still names an installed app (GNOME Settings' `maybe_add_app_id`), by name; per app `{id, canonical, path, name (WinPoint's language, note 8), icon, usesNotifications, listed, values: {the six keys}, locked}` (at most 200) |
| `notifications.app.set {appId, values}` | yes | amendment 6: `gsettings::write_values_in` with the row `Service("notifications.app.set")` writes the relocatable child at `/org/gnome/desktop/notifications/application/<canonical>/` with `application-id` = `<id>.desktop` and the values (one or more of enable, enable-sound-alerts, show-banners, force-expanded, show-in-lock-screen, details-in-lock-screen, booleans only), then the canonical id is added to `application-children` (a `Read` row for `gsettings.set`; only this method adds to it, as GNOME Shell's `NotificationApplicationPolicy.store()` does, and not while the key is locked). An id no installed app has and GNOME Shell never recorded is `not_found notifications.appNotFound`. Event `notifications.changed {what: "app", appId, canonical}` |
| `background.catalog {}` | no | GNOME's chooser: `gnome-background-properties/*.xml` of the system data dirs, then the user's (cc-background-xml.c's order, files by name, doubles by (file, dark file) once), `deleted="true"`, `(none)` and missing files left out, a missing dark twin dropped, names in WinPoint's language (nl, nl_NL, nl_*; en_GB, en, the untranslated one), `.xml` slideshows with their first picture as `preview`; `own` = the regular files in `~/.local/share/backgrounds`, newest first (bg-recent-source.c), at most 200 |
| `background.add {path}` | yes | GNOME's Add Picture: a picture (GIO's content type `image/*`, at most 256 MB) copied into `~/.local/share/backgrounds` as `<YYYY-mm-dd-HH-MM-SS>-<name>` (`-2-`, `-3-` ... when taken; `create_new`, never over a file); a picture already in that folder is answered as it is (`copied: false`). Event `background.changed {what: "added", path}` |
| `background.remove {path}` | yes | a file directly in that folder (no symlink, the folder resolved) moved to the trash, as GNOME Settings does; anything else `denied background.notOwn`. Event `background.changed {what: "removed", path}` |

Every call the host passes to notificationsBridge.js now also carries `sounds` (settings.json `notifications.sounds`)
and `toasts` (`notifications.showToasts`), next to the existing `dnd`.

Allow rows: none new; B0's rows in `allow/apps.rs` are what the pages use (notifications `show-banners` Mirrored,
`show-in-lock-screen` Write, `application-children` Read, the relocatable application schema `Service`, the look keys
`accent-color` and `org.gnome.desktop.background` Owned). Stems: `notifications.badApp`, `notifications.appNotFound`,
`background.{badArgs, notFound, notAnImage, tooLarge, copyFailed, notOwn, removeFailed}` (`#@ new`, listed in
`tests/i18n/BASELINE-CHANGES.rust-settings-f`).

### Extension (amendments 4, 63)

`extension/notificationsBridge.js`: when it hands a notification to the ready consumer for a toast (not silent),
the record is marked; when the consumer acknowledges it (`notifications.delivered`) the bridge calls
`notification.playSound()` once (it checks the app's `enable-sound-alerts` itself and plays the sound the
notification carries), unless `notifications.sounds` or `notifications.showToasts` is false or Niet storen is on.
The native shell sends `consumerReady` and `delivered` straight to the extension (native/shell/src/gnome.rs
`send`), not through a host, so its calls carry only `{pid, ready, dnd}` / `{id, pid}`: the bridge reads
`notifications.*` from WinPoint's settings.json (the file extension.js's LanguageWatch follows) when a sound is due,
and a call's own `sounds` / `toasts` / `dnd` (a host's call) win over the file. An update of the same
notification is acknowledged already, so it is silent and plays nothing again. A source whose policy is turned off
(`enable` false) reaches neither the toast nor the list (GNOME's tray drops it too; the bridge also skips a record
made before), and `policy.forceExpanded` (or a critical notification) travels as `expanded: true` in every item.
The plan's "drop the sound before GNOME plays it" step is gone, as the amendment says.

### Native

* `pages/notifications.rs`: Windows' order. The Meldingen expander (switch: `notifications.showToasts`) holds
  "Meldingen weergeven op het vergrendelingsscherm" (`st-notifications-show-in-lock-screen`, GNOME's key) and
  "Toestaan dat meldingen geluiden afspelen" (`notifications.sounds`), amendment 5; Niet storen is GNOME's
  `show-banners` inverted (`st-notifications-show-banners`; settings.json's toggle only where the schema is missing)
  with the notice while it is on; "Meldingen van apps en andere afzenders" with Sorteren op (Meest recent: the
  notification centre's newest per app first, then by name; Naam; remembered in settings-app.json) and one row per
  app (`notifications-app-<canonical>`: icon, name, Windows' line "Banners, Geluiden", the switch
  `st-notifications-<canonical>-enable` through `notifications.app.set`, the row opens `app-notifications {id}`;
  the first 80 apps follow outside writes); Testmelding versturen stays at the end.
* `pages/app_notifications.rs` (data page `{id: desktop id}`, title = the app's name from apps.list; also for an app
  that is not in notifications.apps, as app-advanced's Meldingen link passes any app): Meldingen (`enable`), a
  Meldingsbanners card with the checkboxes Meldingsbanners weergeven (`show-banners`) and Inhoud weergeven in banners
  (`force-expanded`), Meldingen weergeven op het vergrendelingsscherm (`show-in-lock-screen`), Inhoud verbergen
  wanneer meldingen op het vergrendelingsscherm staan (`details-in-lock-screen` inverted), Een geluid afspelen
  wanneer er een melding binnenkomt (`enable-sound-alerts`); GNOME's sensitivity (all need Meldingen; Show Content
  needs the banners; the lock-screen rows need Meldingen' own lock-screen row, the content row also the app's);
  a line when the lock screen or WinPoint's sounds are off for all apps; Gerelateerde instellingen: Meldingen and
  the app's Geavanceerde opties. Read and followed through Gs at the app's path, written through
  `notifications.app.set`; control names `st-app-notifications-<key>`.
* `pages/personalization.rs`: unchanged (plan "keep": the preview, the themes and the registry's child links).
* `pages/background.rs`: inside "Je achtergrond aanpassen" (Afbeelding): Recente afbeeldingen (the user's copies,
  newest first, each with GNOME's remove button `background-remove-<i>` → `background.remove`, then the current
  picture and the photos chosen before; at most five; `wp-own-<i>`), Een foto kiezen · Door foto’s bladeren (GTK's
  file dialog, then `background.add`, the copy becomes `wallpaper.path`; an older host without background.* keeps
  the photo itself), Afbeeldingen (WinPoint's own), Afbeeldingen van Ubuntu en GNOME (`background.catalog`,
  `wp-gnome-<i>`, a slideshow with the badge Diavoorstelling; a click sets `wallpaper.path` to the file of the
  current mode, of a pair the dark twin in dark mode, and `wallpaper.fit` from the item's options). The grids reload
  on `background.changed`. Removing the current picture sets WinPoint's default (`builtin:lotus-dark`).
* `pages/colors.rs`: in the Accentkleur expander the row "Accentkleur in Ubuntu- en GNOME-apps" shows GNOME's
  `accent-color` (`st-colors-accent-color`, read-only: WinPoint's look owns it) by GNOME's name and follows it, with
  "Ubuntu- en GNOME-apps gebruiken de dichtstbijzijnde GNOME-accentkleur" (or "... houden hun eigen accentkleur"
  while theme.syncSystem is off).
* UI strings: `tests/i18n/windows-glossary/f.tsv`, section "slice 2" (45 rows). Windows' "Recent images" carries
  `// glossary-ok: Afbeeldingen` (glossary.tsv maps Afbeeldingen to Pictures).

### Harness

`tests/gnome/scenarios/settings-apps.py` gains three parts (nl: `settings-apps.out`, en: `settings-apps.en.out`):
Meldingen (the lock-screen checkbox and Niet storen clicked into the private keyfile, the mirror's
`notifications.dnd`, both following outside writes; Bestanden in the app list with its switch; its row opens its
page, whose "Een geluid afspelen ..." writes `enable-sound-alerts` at Bestanden's path together with
`application-id` and `application-children`, and follows an outside write); amendment 63 (GNOME notifications of
Bestanden made in the shell through `org.gnome.Shell.Eval`, `MessageTray.Sound.prototype.play` counted: exactly one
sound while the WinPoint toast shows; none with `notifications.sounds` false, none with Niet storen, none with the
app's `enable-sound-alerts` false); Persoonlijke instellingen, Achtergrond (a click on the first Ubuntu picture sets
`wallpaper.path` and GNOME's `picture-uri`; `background.add` copies a picture, its tile and remove button appear,
the click trashes it) and Kleuren (GNOME's accent shown).

### Runs (16-11, worktree f, on 7459d2ce + this slice)

* `wt/host-tests.sh f --lib`: 597 passed, 1 failed, 19 ignored, 0 temp entries left. The failure is slice 1's note 7
  (`our_binaries_equal_readelf`, B0's libm). New: 7 in `services::notifications::app_settings_tests`, 3 in
  `services::background::tests`.
* `wt/native-tests.sh f`: 70 passed, 0 failed, 1 ignored (win11-gtk-settings 35, of them 7 new); no warning in the
  settings crate.
* `wt/build.sh f all` / `host` / `native`: ok, native-release refreshed (19 binaries).
* `wt/checks.sh f`: as at the base (rustfmt 0 failing; check_rust 0 errors, 57 warnings; drift 0 errors, 15
  warnings; native 0 errors, 48 uncovered; test_native OK; release OK; installer 70 as at the base).
* Harness `settings-apps.py`: nl 63 checks, en 72 checks, none failed. The first runs failed three checks, all
  fixed: the sound with `notifications.sounds` off (the shell's calls bypass the host: the bridge now reads
  settings.json), the app page not following an outside write (note 7 below: the page's `Gs` was dropped), and a path
  check that assumed `~/.local/share` (the harness's data dir is `run/data`).

### plan.json controls dropped or changed

* notifications, "Toestaan dat meldingen geluiden afspelen": its consumer is amendment 4's sound in the bridge
  (no dropping of a GNOME sound, which never played).
* notifications, Niet storen: GNOME's key with the mirror (the plan), and the existing settings.json toggle only as
  the fallback where the schema is missing.
* app-notifications: Windows' rows are toggles in cards (Windows 11 25H2: Meldingen, then the banner card with
  checkboxes, then toggle cards); the plan says checkbox for five of them. The banner pair are checkboxes as in
  Windows; GNOME's Lock Screen › Show Banners is a toggle card of its own (Windows has none). Windows' "Meldingen
  weergeven in het meldingencentrum" and the priority radio have no GNOME key and are left out.
* background, the catalogue's light and dark pairs: WinPoint's `wallpaper.path` holds one file, so a pair sets the
  file of the current mode; following a later mode switch needs the wallpaper.rs/twin_for change of note 1 below.
  `pcolor`/`scolor` of an item are not written (WinPoint's own colour stays `wallpaper.color`).
* background, Windows' Diavoorstelling (album, interval, shuffle) is not in the plan; GNOME's timed slideshows are
  catalogue items (none is installed here: `resolute-wallpapers.xml` names `/usr/share/backgrounds/resolute.xml`,
  which this machine lacks, so it is left out as GNOME Settings leaves out a missing file).
* colors: no control changed; GNOME's OLED hint (hidden in GNOME) is left out.

### Notes for I (slice 2)

1. **Light and dark pairs follow the mode** (`host/src/wallpaper.rs`, `native/apps/settings/src/common.rs`
   `twin_for`/`resolve_wallpaper_path`, `native/shell/src/desktop/wallpaper.rs` `resolve_path`; none is F's): a new
   settings key `wallpaper.pathDark` (`host/src/settings-defaults.json`: `""`). `background.rs`'s `choose_item` then
   writes `{"path": item.path, "pathDark": item.dark or ""}` instead of the file of the current mode (F may make that
   one-line change once the key exists); wallpaper.rs `target()` takes `pathDark` for the dark URI when it is set
   and `followTheme` is on, and both twin helpers pick it in dark mode.
2. **Slideshows on WinPoint's desktop** (`native/shell/src/desktop/wallpaper.rs`, multi-monitor's): a GNOME
   slideshow `.xml` as `wallpaper.path` reaches GNOME's `picture-uri` as GNOME Settings writes it, but the WinPoint
   desktop decodes the file as a picture. Patch: in `read()`/`load()`, when the path ends in `.xml`, use the
   slideshow's current `<static><file>` (gnome-desktop's gnome-bg-slide-show.c: the starttime plus the durations,
   modulo the total) or, simplest, its first one (`background::slideshow_preview` does the first). No slideshow is
   installed on this machine, so nothing shows it today.
3. **`expanded` in the shell** (`native/shell/src/flyouts/notifications.rs`, no batch's): GNOME's "Inhoud weergeven
   in banners" (force-expanded) arrives as `expanded: true`. Patch: in `Toasts::present` the body's
   `wrapped(&body, &["nc-body", "nc-text-2"], 3)` becomes `wrapped(&body, &["nc-body", "nc-text-2"], if n["expanded"]
   == true { 12 } else { 3 })`, and in the centre's card (`fo-notif-body`) `4` becomes `if n["expanded"] == true { 12
   } else { 4 }`.
4. **Contract (§5.7):** `notifications.changed` is new (the contract calls it existing); its data is `{what: "app",
   appId, canonical}`. `application-children` stays a `Read` row: `notifications.app.set` adds to it through the
   settings object itself after `write_values` took the child (a Read row is refused for every caller of
   `write_values`).
5. **Canonical ids:** GNOME Shell collapses `--` (`_canonicalizeId`), GNOME Settings' `g_strcanon` does not. The host
   follows the shell (the reader of the keys); both agree for every desktop id without two such characters in a row.
6. **DND and critical notifications:** amendment 4 says no sound while DND is on; GNOME would play a critical
   notification's sound with its banner. The bridge follows the amendment (a critical toast in DND is silent).
7. **B0 helper (`native/apps/settings/src/gs.rs`, frozen):** `Gs::load`'s `gsettings.changed` handler holds the
   items only weakly, so a `Gs` whose clones are all dropped stops following; the bound controls keep one in their
   handlers, but a page that only calls `gs.follow` (the notification list, the app page, Kleuren' accent row) lost
   every outside change (the first harness run showed it). F's pages now keep a clone until the page goes
   (`p.on_cleanup(move || drop(keep))`). Patch for I so no page has to: in `Gs::load`, after `p.on("gsettings.changed",
   ..)`, add `let keep = gs.clone(); p.on_cleanup(move || drop(keep));`.
8. **App names:** the notification list takes an app's name from apps.list, else from its desktop file in
   WinPoint's language (`Name[nl]` / `Name[en_GB]`, notifications.rs `name_in`). GNOME's own NoDisplay panel apps
   (gnome-color-panel, ...) translate through `X-GNOME-Gettext-Domain`, so they keep the host's locale ("Color
   Management" in a Dutch WinPoint in the harness); a gettext lookup in another language needs the process locale.
9. **Optional, the shell (`native/shell/src/flyouts/notifications.rs`):** `heartbeat()` could pass `"sounds":
   shell.setting("notifications.sounds") != false, "toasts": shell.setting("notifications.showToasts") != false` and
   `arrived()`'s `notifications.delivered` the same, so the bridge needs no file read; it already prefers a call's
   values.

## Slice 3: Privacy en beveiliging, Apparaatbeveiliging, Diagnostische gegevens en feedback, Activiteitengeschiedenis, Zoekmachtigingen, Deze pc doorzoeken, Locatie, Camera, Microfoon

### Host

`host/src/services/privacy.rs` (METHODS, dispatch, `Targets` with the system daemons' names, the system bus seam and
the ubuntu-insights seam) + `privacy/{reports,insights,security,search}.rs`; fragment `host/src/api-methods/apps.json`;
stems `host/i18n/privacy.tsv` (7 rows, `#@ new`, listed in `tests/i18n/BASELINE-CHANGES.rust-settings-f`).

| method | dangerous | what |
|---|---|---|
| `privacy.apps.sandboxed {}` | no | every installed Flatpak and Snap with its portal app id (`<flatpak id>`, `snap.<name>`), desktop id, name in WinPoint's language and icon: the permission pages name the PermissionStore's apps with it |
| `privacy.reports.get {}` | no | whoopsie-preferences (system bus `com.ubuntu.WhoopsiePreferences`, `GetAll`): `{available, mode: never\|auto\|manual, reportCrashes, automatic}` (cc-diagnostics-panel.c's mapping); `available: false` without the daemon |
| `privacy.reports.set {mode}` | yes | `SetReportCrashes(b)` then `SetAutomaticallyReportCrashes(b)`, both through `call_interactive` (polkit `com.ubuntu.whoopsiepreferences.change`, YES for sudo users by Ubuntu's rules); `unsupported privacy.reportsUnavailable` without the daemon; event `privacy.changed {what: reports, mode}` |
| `privacy.insights.get {}` | no | amendment 38: `ubuntu-insights consent <source>` (tools::system_tool, LC_ALL=C) per source (`linux`, `ubuntu_release_upgrader`): `{available, sources: {..: true\|false\|null}}`; null = no consent file yet (the tool exits 1, GNOME shows off). The CLI's output (`<source>: true`) and file (`~/.config/ubuntu-insights/<source>-consent.toml`, `consent_state = true`) were verified with 0.9.3ubuntu0.1 in a scratch `--consent-dir` |
| `privacy.insights.set {source, consent}` | yes | `ubuntu-insights consent <source> --state true\|false`; `failed privacy.consentFailed` with the tool's message; event `privacy.changed {what: insights, source}` |
| `privacy.insights.report {source?: linux}` / `{cancel: true}` | no | GNOME's Show Report: `ubuntu-insights collect --dry-run` (prints the platform report as JSON, writes no cache, uploads nothing), indented; one at a time (`busy privacy.reportBusy`); `{cancel: true}` stops a running one (Cancel Report Generation; the run answers `cancelled privacy.reportCancelled`); 90 s at most; `ubuntu_release_upgrader` needs a metrics path the CLI cannot make, so only the platform report (as GNOME) |
| `privacy.security {}` | no | fwupd (system bus `org.freedesktop.fwupd` `/`): `HostSecurityId` parsed into `level` and `runtimeIssue` (the `!`), `GetHostSecurityAttrs` and `GetHostSecurityEvents(10)` as `{appstreamId, title, level, result, previous, flags, success, runtimeIssue, obsoleted, created, plugin}`, `secureBoot` (on/off/problem from `org.fwupd.hsi.Uefi.SecureBoot`), `daemonVersion`, `vendor`, `product`, `securityCenter` (installed); read only, never interactive |
| `privacy.securityCenter.open {}` | no | Ubuntu's Security Center (`desktop-security-center_desktop-security-center.desktop` or `security-center_security-center.desktop`): `launch.app` through the bridge (GNOME Shell's child); while the bridge cannot (panelBridge.js does not route `launch.*` yet, §8.1) `apps.launch` as GNOME Settings starts it; `not_found privacy.securityCenterMissing`; a dry run starts nothing |
| `privacy.searchProviders {}` | no | `gnome-shell/search-providers/*.ini` of the system data dirs (GNOME Shell's and GNOME Settings' dirs; the first dir naming a DesktopId wins, files by name), only providers whose app is installed: `{providers: [{desktopId, name, icon, defaultDisabled}]}` by name |

Allow rows: none new. The pages use B0's rows in `allow/apps.rs`: privacy `remember-recent-files` (Mirrored),
`recent-files-max-age`, `disable-camera`, `disable-microphone`, search-providers `*`, Tracker
`index-recursive-directories` / `index-single-directories` (optional tracker), location `enabled`. One allow-host row:
`privacy/search.rs` `Shell Search Provider` (the keyfile group, never shown).

Tests (`host/src/services/privacy/tests.rs`, 4 new, 18 in the module): the pure parts (the drop-down's mapping,
`ubuntu-insights` consent output and the report's JSON, HSI ids, fwupd attributes and Secure Boot, the provider files
of two data dirs with doubles, wrong groups and bad ids); ubuntu-insights against a stub script that logs its argv
(get per source, a dry run runs no write, the write and its event, a refused write, the report, Annuleren of a running
report and busy meanwhile, not installed); a whoopsie-preferences fake with properties on the sandbox's private bus
(get, a dry run sends no Set, Automatisch sends both Sets in GNOME's order with ALLOW_INTERACTIVE_AUTHORIZATION, never,
AccessDenied → `denied core.notAuthorized`, no daemon); a fwupd fake (level, runtime mark, attributes, events with
their previous result, `GetHostSecurityEvents(uint32 10)`, nothing interactive, no daemon). The test `Fake` gained
properties (`Fake::start_with`). The bad-arguments test covers the new methods.

### Native

* `pages/privacy.rs`: Windows' groups from the registry: Beveiliging (Apparaatbeveiliging), Machtigingen
  (Diagnostische gegevens en feedback, Activiteitengeschiedenis, Zoekmachtigingen, Deze pc doorzoeken),
  App-machtigingen (Locatie, Camera, Microfoon and Windows' Meldingen row → Systeem › Meldingen), then "Zo houdt
  WinPoint je veilig". The recent-files rows moved to Activiteitengeschiedenis (plan.json).
* `pages/privacy_history.rs`:
  * activity: "Mijn activiteitengeschiedenis op dit apparaat opslaan" (`st-activity-remember-recent-files`, GNOME's
    key, the mirror keeps `privacy.recentFiles`; settings.json's toggle only without the schema); "Geschiedenis
    bewaren gedurende" (`st-activity-recent-files-max-age`: 1 dag, 7 dagen, 30 dagen, Altijd, plus a current value of
    its own; greyed while history is off, as GNOME); Activiteitengeschiedenis wissen · Geschiedenis wissen
    (`activity-clear`, apps.clearRecent after the question).
  * searchpermissions: the line that this is GNOME's overview search and Start uses WinPoint's index (amendment 45);
    "Zoekresultaten van apps opnemen" (`st-searchpermissions-disable-external`, inverted); Zoekresultaten: one row per
    provider in `sort-order` (GNOME's order: listed ones first), the switch `st-searchpermissions-<disabled|enabled>-<id>`
    (`disabled`, or `enabled` for a DefaultDisabled provider, as app_advanced.rs's Zoeken), Omhoog / Omlaag
    (`searchpermissions-up-<id>` / `-down-`) write the whole `sort-order` as GNOME does; the list greys while app
    search is off and follows outside writes; Gerelateerde instellingen: Deze pc doorzoeken.
  * searching (needs caps.tracker): Mijn bestanden zoeken, Klassiek / Uitgebreid (`searching-classic`,
    `searching-enhanced`). Amendment 45: each change calls index.setRoots first and then writes Tracker's two lists in
    one `gsettings.set` (a refused Tracker write puts the index's folders back). Uitgebreid = WinPoint's index over
    every drive (`alles.roots` "auto", its default) and Tracker's `$HOME` recursive (localsearch 3.11's default);
    Klassiek = the index on the search locations that are on (with subfolders) and Tracker without `$HOME` recursive,
    with GNOME's default places (Persoonlijke map without subfolders, the XDG folders) when none is on. Zoeklocaties
    aanpassen (`searching-locations`): GNOME's Search Locations: Standaardlocaties (Persoonlijke map → 
    `index-single-directories`; Bureaublad, Documenten, Downloads, Muziek, Afbeeldingen, Video's → recursive, written
    as Tracker's tokens `&DESKTOP` ...), Bladwijzerlocaties (`~/.config/gtk-3.0/bookmarks`, local folders), Aangepaste
    locaties (other entries of either list, Verwijderen, Locatie toevoegen through GTK's folder dialog); switches
    `searching-place<path>`; in Klassiek a switch also changes the index's folders. Rebuilt on every Tracker change.
    Mappen uitsluiten van uitgebreid zoeken: the index's own `excluded` (index.setRoots with the roots it has).
    Gerelateerde instellingen: Zoekmachtigingen, Geavanceerde indexeringsopties (opens Alles).
* `pages/privacy_perms.rs` (location, camera, microphone): the device switch (`st-location-enabled`,
  `st-camera-disable-camera`, `st-microphone-disable-microphone`, the last two inverted); the expander "Apps toegang
  geven tot je ..." with GNOME's Permitted Apps (privacy.permissions; names from privacy.apps.sandboxed by portal id,
  else apps.list by `<app>.desktop`, as GNOME Shell stores desktop apps; an app nothing names is left out, as GNOME;
  location shows "Laatst geopend op" from the entry's timestamp), the switch `<page>-app-<id>` writes
  privacy.setPermission, greyed while the device switch is off; empty and unavailable lines; the info row "Desktop-apps
  toegang geven tot je ..." (plan: info only). Locatie says when GeoClue is missing (caps.geoclue).
* `pages/privacy_security.rs`:
  * security: Apparaatbeveiliging (fwupd): the security level as an expander (`security-level`: GNOME's Controles
    mislukt / geslaagd / Beschermd / niet beschikbaar, the runtime line, "Niveau N"), inside every check by
    Beveiligingsniveau 1-4 and "Tijdens het gebruik" (obsoleted ones left out; names from GNOME's appstream ids in
    WinPoint's language, fwupd's own title otherwise; Geslaagd/Mislukt with fwupd's result) and Technisch rapport
    kopiëren (`security-copy-report`: GNOME's report, in WinPoint's language); Windows' Beveiligd opstarten
    (`security-secure-boot`) and Beveiligingsprocessor (TPM 2.0) rows; Beveiligingsgebeurtenissen; Meer beveiliging:
    Beveiligingscentrum openen (`security-center`, when installed) and USB4- en Thunderbolt-apparaten (the usb page,
    caps.thunderbolt).
  * diagnostics: the expander Diagnostische gegevens with Ubuntu's two system reports
    (`st-diagnostics-insights-linux`, `st-diagnostics-insights-ubuntu-release-upgrader`); Diagnostische gegevens
    weergeven · Rapport weergeven (`diagnostics-view`: a dialog that makes the report, closing it cancels);
    Foutrapporten naar Canonical verzenden (`st-diagnostics-reports`: Nooit / Automatisch / Handmatig, reverted when
    refused).
* UI strings: `tests/i18n/windows-glossary/f.tsv`, section "slice 3" (173 rows). The Security Center is named as its
  snap's desktop file names it (`Name[nl]` Beveiligingscentrum, `Name[en_GB]` Security Centre). "Mijn
  activiteitengeschiedenis op dit apparaat opslaan" is Windows' row and carries `// glossary-ok: Opslaan` (glossary.tsv
  maps Opslaan to Save).

### Harness

`tests/gnome/scenarios/settings-privacy.py` (nl `settings-privacy.out`, en `settings-privacy.en.out`) with the
stand-ins of `tests/gnome/lib/standins/f.py` (whoopsie-preferences and fwupd on the private system bus, a
PermissionStore on the private session bus). It covers, with real clicks and the private keyfile: the overview's
links; Activiteitengeschiedenis (remember-recent-files and the mirror's privacy.recentFiles, the duration greyed and
written as 7, an outside write followed); Zoekmachtigingen (Bestanden's provider into `disabled` and back from
outside, Omhoog writing `sort-order` with Bestanden first, App search off greying the list); Deze pc doorzoeken
(Klassiek: settings.json `alles.roots` the XDG folders and Tracker's lists without `$HOME`; the Muziek switch out of
both; an outside Tracker write shown; Uitgebreid: `alles.roots` "auto" and `$HOME` first); Locatie, Camera
(Bestanden from the store stand-in, SetPermission(devices, camera, org.gnome.Nautilus, ['no']) without interactive
authorization, greyed while Cameratoegang is off, an outside write followed) and Microfoon; Apparaatbeveiliging (HSI:1!
from fwupd's stand-in: "Controles geslaagd" with the runtime line, the reads not interactive); Diagnostische gegevens
en feedback (Automatisch: both Sets with interactive authorization; the consent switch runs `ubuntu-insights consent
linux --state true` through system_tool and goes back when the run's stub fails; Rapport weergeven runs `collect
--dry-run` and shows the error); every page's screenshot and, in English, no Dutch words.

### Runs (16-11, worktree f, on 19ad65ed + this slice)

* `wt/host-tests.sh f --lib`: 601 passed, 1 failed, 19 ignored, 0 temp entries left. The failure is slice 1's note 7
  (`our_binaries_equal_readelf`, B0's libm). New: 4 in `services::privacy::tests` (18 there).
* `wt/native-tests.sh f`: 76 passed, 0 failed, 1 ignored (win11-gtk-settings 41, of them 6 new); no warning in the
  settings crate.
* `wt/build.sh f all`, then `native`: ok, native-release refreshed (19 binaries).
* `wt/checks.sh f`: as at the base (rustfmt 0 failing; check_rust 0 errors, 57 warnings; drift 0 errors, 15 warnings;
  native 0 errors, 48 uncovered; test_native OK; settings-pages OK; release OK; installer 70 as at the base).
* Harness `settings-privacy.py`: nl 59 checks, en 68 checks, none failed. The first runs failed: the scenario's
  `from lib.standins import ...` (no `lib` module under `python3 -I`: it loads the package by path now, note 7), the
  Muziek switch inside the collapsed Zoeklocaties expander (the scenario opens it first), and the system reports
  shown as unavailable (note 8).

### plan.json controls dropped or changed

* privacy: unchanged links; Meldingen added to App-machtigingen (Windows' row; it opens Systeem › Meldingen).
* activity: "Geschiedenis bewaren gedurende" has GNOME's presets (1, 7, 30 dagen, Altijd) plus a current value of its
  own, and is greyed while history is off (GNOME).
* searchpermissions: Omhoog / Omlaag buttons per row instead of GNOME's drag handle (GNOME also has Move Up / Move
  Down in a menu); Windows' SafeSearch, cloud content, search history and highlights are left out (no GNOME function).
* searching: Windows' "Mijn bestanden zoeken" (Klassiek / Uitgebreid, windows.json) is the coupling amendment 45 asks
  for: the index first, Tracker the same folders; GNOME's three location groups are inside "Zoeklocaties aanpassen";
  "Mappen uitsluiten van uitgebreid zoeken" is the index's own `excluded` (Tracker's ignored-directories take names,
  not paths, and have no GNOME control). Left out: Windows' Indexeringsstatus (index.status answers only in Alles'
  own host: in Settings it says "Open Alles om bestanden te doorzoeken"), "Energie-instellingen respecteren tijdens
  indexeren" (Tracker `index-on-battery`: no GNOME control, no allow row). The Persoonlijke map without subfolders is
  not mirrored into the index (the index has no "without subfolders").
* location / camera / microphone: Windows' "Apps toegang geven tot je ..." expander has no switch of its own (GNOME has
  one key per device, which is the top switch); Recente activiteit, Melden wanneer apps locatie aanvragen,
  Standaardlocatie and Locatiegeschiedenis are left out (windows.json: no GNOME function); `max-accuracy-level` has
  no control (GNOME has none either), so its allow row is Read since review fix 1 (it was in the Write row with
  `enabled`).
* security: Windows' Kernisolatie is left out (fwupd's attributes cover IOMMU and the like in the checks list);
  fwupd's fix/undo of an attribute (polkit `fix-host-security-attr`) is not offered (GNOME 50's panel offers none);
  Thunderbolt is the link to Bluetooth en apparaten › USB (batch B's page).
* diagnostics: Windows' Feedbackfrequentie, Op maat gemaakte ervaringen and Diagnostische gegevens verwijderen are left
  out; GNOME's ABRT row is Fedora's (not on Ubuntu). The report dialog has no separate Annuleren: closing it cancels.

### Notes for I (slice 3)

1. **Security Center (§8.1):** panelBridge.js does not route `launch.*` to launchBridge.js yet, so
   `privacy.securityCenter.open` falls back to `apps.launch` (a child of the Settings host, as GNOME Settings starts
   it as its own child). Once §8.1 is applied the bridge path is taken; nothing in F changes.
2. **Contract additions (§5.7):** `privacy.insights.report {cancel: true}` (GNOME's Cancel Report Generation) and the
   answer fields `privacy.security.securityCenter` (installed); `privacy.apps.sandboxed` is used by the three
   permission pages to name the store's apps.
3. **Registry (frozen):** `privacy`'s description "Recente bestanden, beveiliging" / "Recent files, security" is stale
   now that the recent-files rows are on Activiteitengeschiedenis: suggest "Apparaatbeveiliging, app-machtigingen,
   activiteitengeschiedenis" / "Device security, app permissions, activity history" with the keywords "locatie",
   "camera", "microfoon", "zoeken" / "location", "camera", "microphone", "search". `searching` needs `tracker`, but
   half of the page (Klassiek / Uitgebreid, the excluded folders) is WinPoint's own index, which works without Tracker:
   I may drop the `needs` (the page then says Tracker is missing and keeps the index rows; `searching()` would need
   its early return split, a small change in F's file).
4. **PermissionStore changes from elsewhere:** GNOME's panels follow the store's `Changed` signal; the host emits
   `privacy.changed` only for its own writes, so a permission changed by the portal's own dialog while a page is open
   shows on the next visit. A relay of `org.freedesktop.impl.portal.PermissionStore.Changed` (for the seven tables of
   `perms::KINDS`) as `privacy.changed {what: permissions, table, id}` would need a signal subscription in a service
   that lives as long as the host (privacy.rs has none), and the three pages would refresh on it. **Superseded:**
   F's own files do this now (review fix 6); nothing for I.
5. **Stand-ins (B0's builder, frozen):** a JSON spec cannot give a `v` value (`aa{sv}`), so fwupd's stand-in answers no
   attributes and the PermissionStore's `Lookup` answers `(a{sas})` without its `v`. Suggest `SERVER` turn
   `{"$v": [sig, value]}` into `GLib.Variant(sig, value)`; the host unit tests cover the attributes meanwhile.
6. **Tracker's `$HOME`:** localsearch 3.11's default is `index-recursive-directories ['$HOME']`; GNOME 50's Search
   Locations binds Home to `index-single-directories`, so GNOME Settings shows Home off while the whole home folder is
   indexed. WinPoint shows the Persoonlijke map on in that case (place_on) and calls it Uitgebreid only when the index
   covers every drive (`alles.roots` "auto"); an index on folders chosen in Alles' options shows as Klassiek, and a
   location switch then replaces those folders with the search locations.
7. **Stand-ins (B0's docstring, frozen):** `tests/gnome/lib/standins/__init__.py` says `from lib.standins import Spec,
   start`, but scenarios run under `python3 -I` and `lib` is no module there. settings-privacy.py loads the package by
   path (`importlib.util.spec_from_file_location(..., submodule_search_locations=[...])`); the docstring (or a
   `load_lib`-like helper for packages on the scenario Helper) should say so for every batch.
8. **Harness stubs (B0's harness.py, frozen):** `prepare()` makes `run/stubs` with the umask (0775 here), and
   host/src/tools.rs takes `WIN11_SYSTEM_TOOL_DIR` only when it is private (no group or world write), so every
   `system_tool` spawn in the harness is refused (the pages then say the tool is not installed). settings-privacy.py
   makes the directory 0700 before Diagnostische gegevens (recorded as `stubs mode before`). Patch for I: in
   `prepare()`, `os.chmod(RUN / 'stubs', 0o700)` after the loop that writes the stubs (tests/rust-sandbox already
   makes its guard-bin private). Batches A, C and E spawn through `system_tool` too.

## Harness step: settings-apps.py and settings-privacy.py in nl and en (16-11)

Both scenarios were rewritten to follow plan.json "testing" (GNOME HARNESS) page by page, and they run under
`--strace`. Every page is opened by its id, or a data page by a real click on its parent (Bestanden from Geïnstalleerde
apps' "…" menu, from its row on Meldingen and from Geavanceerde opties' Meldingen). The header is checked against the
registry's titles (`h.session.titles`; a data page ends in the app's name: "Apps › Geïnstalleerde apps › Bestanden",
"Systeem › Meldingen › Bestanden"). Each page gets a screenshot `<page>.<lang>` and, in English, a check for Dutch
words. Every control is pressed with the real pointer: a page is first scrolled with the real wheel (`reveal`: the
harness's Clutter virtual pointer, `notify_discrete_scroll`) until the control is inside the page's scroll view. The
scenario's last check lists any click that fell back to the dev channel; in the final runs there were none.

### What the scenarios check

* **settings-apps.py**
  * Apps: its three links.
  * Geïnstalleerde apps: Sorteren op Naam (Z-A) turns the list round.
  * Geavanceerde opties of Bestanden:
    * App-machtigingen of a desktop app shows Meldingen and Zoeken and no PermissionStore rows (GNOME shows those
      for Flatpak and Snap; the sandbox has neither).
    * Zoeken is bound to search-providers `disabled` and follows an outside write.
    * Wordt uitgevoerd bij aanmelden writes GNOME's autostart copy (`X-GNOME-Autostart-enabled` true, then false).
    * Standaardkoppelingen: Als standaard instellen and Type verwijderen go into the private `mimeapps.list` (Default
      Applications / Removed Associations), and Opnieuw instellen takes both out again.
    * Beëindigen says Bestanden is not running.
  * Standaard-apps:
    * The type search's chooser writes `text/html` and Teksteditor writes `text/plain` into `mimeapps.list`.
    * Alle standaard-apps opnieuw instellen takes both out.
    * The file-type view and an app's view.
  * Meldingen:
    * The lock screen and Niet storen, with the mirror's `notifications.dnd`.
    * Bestanden's switch in the app list writes `enable` with `application-id` and `application-children`
      (amendment 6) and follows an outside write.
    * Sorteren op Naam sorts by name.
    * WinPoint's own Meldingen and geluiden switches write settings.json.
  * Meldingen van app: each of GNOME's six keys at the app's path, by a click and back from outside, greyed as GNOME
    greys them. That includes the lock-screen rows while the global lock-screen key is off.
  * Amendment 63: the sound cases as before.
  * Achtergrond:
    * Uitrekken writes picture-options `'stretched'`.
    * GNOME's background keys are WinPoint's: `gsettings.get` reports `mode: owned` with setting `wallpaper`, and
      `gsettings.set` refuses them with `denied gsettings.ownedByWinPoint`.
  * Kleuren:
    * Windows' swatch #E81123 sets `theme.accent`, the mirror writes GNOME's `accent-color` `'red'`, and the row
      shows "Rood".
    * The row follows an outside write.
    * `accent-color` is owned by `theme.accent`, and `gsettings.set` refuses it.
  * No pkexec, apt or pkcon is called.
* **settings-privacy.py**
  * Without the daemons, Camera says the app permissions are not available, Apparaatbeveiliging says "Controles niet
    beschikbaar", and Diagnostische gegevens has no Foutrapporten row. With their stand-ins all three appear.
  * Deze pc doorzoeken is shown exactly when `caps.tracker` is true. Beveiligingscentrum and the USB4 link are shown
    exactly when `caps.securityCenter` / `caps.thunderbolt` are true; both are false in the sandbox, so both are
    hidden.
  * Activiteitengeschiedenis: an outside `recent-files-max-age` 30 shows "30 dagen". Geschiedenis wissen empties
    `recently-used.xbel` and keeps a `.bak`.
  * Zoekmachtigingen: after Omhoog, Bestanden is first and has no Omhoog; Omlaag puts it second; an outside
    `disable-external` false turns app search and the list back on.
  * Locatie, Camera and Microfoon each:
    * show Bestanden with the stored value;
    * send SetPermission with GNOME's values (Locatie keeps its timestamp: `['NONE', '1763280000']`), without
      interactive authorization;
    * grey the apps while the device is off, and follow outside writes.
    * Locatie also shows "Laatst geopend op".
  * Apparaatbeveiliging with fwupd's attributes:
    * Niveau 1 and the runtime line;
    * Beveiligd opstarten on, the security processor, the Secure Boot event;
    * every check under its level with its verdict;
    * Technisch rapport kopiëren puts GNOME's report on the clipboard (read back through `St.Clipboard`).
  * Diagnostische gegevens shows the daemon's mode (Handmatig).

### Stand-ins (`tests/gnome/lib/standins/f.py`)

`f.py`'s `start` is B0's `start` with two additions to its server: `byArgs`, which answers per call by the arguments'
GVariant text, and `{"$v": [sig, value]}` for a `v`. The patch is applied to B0's SERVER text and raises when that
text changes; B0's builder itself is unchanged. With these:

* The PermissionStore answers Bestanden per table as GNOME stores it: location `['EXACT', '1763280000']`, camera
  `yes`, microphone `no`.
* fwupd serves five real `aa{sv}` attributes and one event, which closes slice 3's note 5 for batch F.

### A page fixed after the first runs (`pages/notifications.rs`)

A real click on a switch inside a button never flipped it: GtkButton claims the press before GtkSwitch does. This hit
two switches:

* Bestanden's switch in the app list opened the app's page instead.
* WinPoint's Meldingen switch in the expander header folded the expander instead.

`pointer_flips(sw)` adds a capture-phase GestureClick to the switch that claims a press on the switch itself and
flips it; the keyboard is unchanged. Both switches use it. Note for I 2 has the kit-wide fix.

### Runs (16-11; worktree f; the final runs on this step's tree, through wt/harness.sh with `--strace`)

| run | checks | failed | known | strace connect / exec | clicks (all real) | kit dialog buttons (dev fallback) |
|---|---|---|---|---|---|---|
| settings-apps nl | 124 | 1 | 1 | ok (334) / ok (591) | 46 | Standaard instellen, Opnieuw instellen |
| settings-apps en | 135 | 1 | 1 | ok (334) / ok (597) | 46 | Standaard instellen, Opnieuw instellen |
| settings-privacy nl | 101 | 1 | 1 | ok (217) / ok (328) | 24 | Wissen |
| settings-privacy en | 111 | 1 | 1 | ok (217) / ok (324) | 24 | Wissen |

* **The one failure per run** is the known check "a pointer click reaches a kit dialog's button": kit `Dialog`'s
  smoke layer claims every press (settings-b.md note 9). The action behind each such button is still checked after
  the dev channel's click: `mimeapps.list` written or emptied, the history emptied.
* **The chooser's radios** have no names for a dev click, so the type chooser keeps its own selection (the current
  default, `firefox.desktop`) and checks that Standaard instellen writes it.
* **Binaries:** the apps runs used `win11-gtk-settings` with both page changes of this step. The privacy runs ran
  earlier, on the binary without them; neither change touches a privacy page.
* **Regression, once in nl** (outside the committed results): native-shell.py 21 checks, 0 failed; lock-unlock.py 5
  checks, 0 failed.
* **Tests after the page changes:**
  * wt/build.sh f native: ok; native-release refreshed, with `win11-gtk-settings` updated.
  * wt/native-tests.sh f: 76 passed, 0 failed, 1 ignored. win11-gtk-settings has 41 of them; no warnings in the
    settings crate.
  * wt/checks.sh f: the same as before. rustfmt has 0 failing files; check_rust 0 errors and 57 warnings; drift 0
    errors and 15 warnings; the native check 0 errors and 48 uncovered; test_native, settings-pages and release
    pass. The installer check exits 70, as at the base.
  * No host file changed in this step.
* **First runs, before these fixes:**
  * apps nl had 19 failures. A chooser dialog stayed open, because a GtkCheckButton's text is its GtkLabel child,
    and that dialog covered every later page. The scenario now closes any open dialog (`dismiss`).
  * apps nl again had 6 failures: the two switches above, and a race in which `h.gsettings_set` rewrote the keyfile
    under the mirror's own show-banners write. The scenario now waits for the mirror (`dnd()`).
  * privacy nl had 1 failure: the dialog click.

### The screenshots against Windows 11

I read every PNG of the four runs (they are gitignored and stay in the worktree's result dirs).

* **Fixed:** Achtergrond's "Kies hoe de afbeelding het scherm vult" is now a row of the "Je achtergrond aanpassen"
  expander, after "Een foto kiezen", as Windows 11 has it. It was a separate card below the expander.
* **Matches Windows, as far as GNOME's data allows:** the breadcrumbs, the cards with title, description and the
  control on the right, the expanders, Gerelateerde instellingen, the Apparaatbeveiliging sections, and the Locatie,
  Camera and Microfoon pages (device switch, the apps expander, the desktop-apps row).
* **Left as is:**
  * Activiteitengeschiedenis keeps "Activiteitengeschiedenis wissen" in a group of its own, with a larger gap above
    it than Windows has.
  * The sandbox has no music or video player, so Standaard-apps per soort offers Firefox for both. That is GIO's
    answer, not a page bug.
* **For I (frozen):** a long last crumb is cut off: "Privacy en beveiliging › Diagnostische gegevens …" in the
  1100 px window. Windows keeps the current page whole and folds the earlier crumbs into "…" (note 3).

### Notes for I (harness step)

1. **Kit dialogs (confirms settings-b.md note 9).** A pointer click never reaches a button of `kit::controls::Dialog`.
   In F's pages that is Standaard instellen in the type chooser, Alle standaard-apps opnieuw instellen and
   Geschiedenis wissen. It is the same for the chooser's radio buttons, so a mouse user cannot pick another app
   there. Fix and re-check as B's note says. Once it is in, the known check "a pointer click reaches a kit dialog's
   button" passes in both scenarios, and the record `chooser radio` in settings-apps.py then names the picked app.
2. **A switch inside a button (kit `SettingsCard::clickable` / `SettingsExpander` header with `set_control`).**
   GtkButton claims the press before GtkSwitch does, so a pointer click flips nothing: it opens the row or folds
   the expander. F's two cases use `notifications::pointer_flips`. The kit-wide fix is in
   `native/kit/src/controls/cards.rs` `set_control` / `add_control`: when `self.inner.root` is a `gtk::Button`, give
   every `gtk::Switch` in the new control the same capture-phase GestureClick (claim on press, flip on release
   inside the switch). Then F can drop `pointer_flips`. Other batches' header switches (A-E) have the same bug.
   B's note 4 covers header buttons.
3. **Breadcrumb (`app.rs` / kit `Breadcrumb`, frozen):** when the crumbs do not fit, Windows folds the leading crumbs
   into "…" and keeps the current page whole. WinPoint ellipsizes the last crumb instead (Diagnostische gegevens
   en feedback at 1100 px).
4. **Stand-in builder (B0, frozen):** f.py's `byArgs` and `$v` answers are a patch of B0's SERVER text, and it
   raises if that text changes. I can move both into `lib/standins/__init__.py` (a `conv()` for `$v`, and
   `byArgs` in `handler`), so every batch gets them; f.py then only keeps its specs. This supersedes slice 3's
   note 5.
5. **Scenario helper (B0, frozen):** `Helper.open('settings:<page>')` relays the page only when it differs from the
   page the helper last opened. After a click or a deep link has moved Settings elsewhere, opening that same page
   again does nothing. F's scenarios reset `h.surfaces['settings'].page` first. Patch: in `open`, relay whenever a
   page is given (`if kind == 'settings' and page:`).
6. **`h.gsettings_set` rewrites the whole keyfile.** An outside write right after a WinPoint change can undo the
   mirror's own write of another key; here the mirror's `show-banners` came back as Niet storen on. F's scenario
   waits for the mirror first (`dnd()`). The README's helper row could say so.
7. **Not testable in the harness, covered by unit tests:** App-machtigingen' PermissionStore rows, Achtergrond and
   Vereiste machtigingen of a Flatpak or Snap (`/var/lib/flatpak` and `/snap` are not in the sandbox, so neither
   kind of app exists there); snapd's connect (`X-Allow-Interaction`); GlobalShortcutsRebind. Also Locatie's GeoClue
   warning: GeoClue's service file is under `/usr`, so `caps.geoclue` is true in the sandbox.
8. Slice 3's note 8 (`run/stubs` 0775) still holds: settings-privacy.py records `stubs mode before` 0o775 and makes
   the directory 0700 itself.

## Review fixes (16-11, after the batch F review)

Ten findings, each read against the code first. All ten held; each fix has a test (a host or native unit test, a
harness check, or both).

| # | finding | verdict | fix | test |
|---|---|---|---|---|
| 1 | `org.gnome.system.location max-accuracy-level` in a Write row although GNOME Settings 50 has no control for it (gnome-a.json "no UI"; §6.1) | real | `allow/apps.rs`: `enabled` stays Write, `max-accuracy-level` is a Read row of its own (still in exactly one table) | host `location_accuracy_is_read_only_as_gnome_settings_has_no_control`: `write_values` for `gsettings.set` and another method is `denied gsettings.readOnly` (dry run), `read_item` is `allowed`, mode `read`, GNOME's `exact`; `enabled` still passes |
| 2 | `calendar_calls_and_sms_are_kinds_with_their_one_type` (apps/defaults.rs) and `add_copies_as_gnome_settings_and_remove_trashes_only_the_copies` (background.rs) took keyfile GSettings alone as the sandbox and then wrote the real `~/.local/share/applications`, `~/.config/mimeapps.list` and Trash | real | `services::privacy::tests::sandbox()` is `pub(crate)` and asks the whole run-tests.sh environment: keyfile GSettings, a session bus not under `/run/user/`, and HOME, XDG_DATA_HOME, XDG_CONFIG_HOME and WIN11_SYSTEM_TOOL_DIR equal to `$SANDBOX/{home,data,config,guard-bin}`; both tests and notifications.rs's own guard use it | host `the_sandbox_guard_needs_all_of_run_tests_environment` (a pure `sandbox_in(var)`: the full environment passes; keyfile with the real HOME, each variable missing or pointing outside, the live bus, a relative SANDBOX all fail); the sandbox runs still run the guarded parts (no "skipped" in the log) |
| 3 | `the_extension_hears_the_sound_and_toast_settings` could not fail (an empty settings.json and `!= false`), and the extension-bound payload was not checked | real | `Notifications::extension_flags(&mut native)` builds what every extension call carries (`dnd`, `sounds`, `toasts`, `language`); `call` uses it | the test writes settings.json with `sounds`/`showToasts` false (both, each alone, neither) and asserts `sounds_and_toasts()` and the payload (`sounds: false, toasts: false`, the call's own `id` kept, the language) |
| 4 | Geavanceerde opties read the device-off state (disable-camera, disable-microphone, location `enabled`) and `disable-external` once | real | `permissions()`: each Locatie/Camera/Microfoon row follows its device key (`gs.follow`: sensitivity and "Uitgeschakeld voor alle apps in Privacy en beveiliging"); Zoeken follows `disable-external` (sensitivity and its line); the page keeps its `Gs` until cleanup | native `permission_rows_follow_the_store_and_the_device_switches` (`device_off`, `device_key`, `search_state`); harness settings-apps: `disable-external` written true from outside greys Zoeken with "Zoeken in apps is uitgeschakeld", false makes it usable again (the device rows need a Flatpak or snap, which the harness has not: note 7 of the harness step) |
| 5 | Meldingen van app: a refused write reverted the control silently, so the page's optimistic `values` kept the refused value and the other rows kept the wrong sensitivity | real | `app_switch` / `app_checkbox` take `changed(value)`: called with the key's value on a click and with the old value after a refused write; the page's copy and `sync()` hang on it (the second `connect_toggled` per control is gone); notifications.rs passes a no-op (its summary follows gsettings.changed only) | native `a_control_stands_for_its_keys_value` (`key_value`, also inverted); harness settings-apps `refused_write`: a test app's page, the app's desktop file removed, a click on Meldingen is refused (`not_found`), the switch comes back on, Meldingsbanners, the lock-screen row and the sound stay usable, nothing is written at the app's path |
| 6 | Locatie, Camera, Microfoon and Geavanceerde opties read the PermissionStore once; nothing followed its `Changed` | real | host: `Privacy::watch_store` (started by `Privacy::new`): one thread with its own main context holds a subscription to the store's `Changed` (`perms::watch`, `perms::changed_kind` for the entries of `perms::KINDS`) and relays it as `privacy.changed {what: "permissions", table, id}` until `shutdown` or the service ends; pages: privacy_perms re-reads the entry on a matching event and updates the switches and "Laatst geopend op" in place (`same_apps`), or builds the page again when an app appeared or went; app_advanced re-reads `privacy.app {sizes: false}` and sets the rows' switches and Achtergrond's choice in place (`BackgroundState::show`), or builds again | host `a_change_in_the_permission_store_from_elsewhere_reaches_the_pages` (the signal's parsing; on the private bus: a table GNOME does not show is passed over, the camera relayed, the same signal from another connection ignored, nothing after `shutdown`); native `a_change_in_the_store_updates_the_rows_or_builds_the_page_again`; harness settings-privacy: the stand-in PermissionStore now keeps SetPermission's values and emits `Changed` (f.py `permissionStore`), the clicked value stays after the store's own Changed, and a SetPermission made by another client while the page is open shows at once on Locatie, Camera and Microfoon |
| 7 | Installed apps' "…" menu took "Openen in {store}" and its enabled state from `store_app()` alone, while the target depends on the app's kind | real | `installed::store_item(info, installed)`: label and enabled from `store_target`; the item is created greyed and filled in place once `privacy.app {sizes: false}` answers (as Verwijderen with software.uninstallInfo) | native `openen_in_store_names_the_store_that_opens_the_app` (App Center only: a system package greyed, a snap enabled; both stores: a Flatpak names GNOME Software; unknown greyed); harness settings-apps: Bestanden (a system package, App Center only) has the item greyed |
| 8 | `privacy.securityCenterMissing` en said "Security Center" (US) under the "Security Centre" card | real | en: "Security Centre isn’t installed." | host `the_security_centre_is_named_in_en_gb` (both languages, and no "Center" outside "App Center" in the en column of privacy, background, notifications and apps.tsv) |
| 9 | `t("App", "App")` (Specificaties) had no windows-glossary row | real | the row App / App / windows / no in f.tsv between Version and Data | a scratch check of every t() pair new in F's pages against windows-glossary/*.tsv and glossary.tsv: only three pairs have no row, and all three existed at af8eebdc (moved from privacy.rs to privacy_history.rs) |
| 10 | Geavanceerde opties' order differed from Windows (Cache wissen inside Specificaties, the GNOME additions between Wordt uitgevoerd bij aanmelden and Beëindigen) | real | `app_advanced::sections(app)`: Specificaties, App-machtigingen, Machtigingen voor apps op de achtergrond, Wordt uitgevoerd bij aanmelden, Beëindigen, Opnieuw instellen (Cache wissen and its question, a group of its own, only with `canClearCache`), Verwijderen, then Algemene sneltoetsen, Vereiste machtigingen, Standaardkoppelingen | native `the_sections_follow_windows_order_then_winpoints_additions`; harness settings-apps: the rows of App-machtigingen, autostart, Beëindigen, Verwijderen and Standaardkoppelingen top to bottom; no Opnieuw instellen for a system package |

Notes:

* The relay also brings the host's own writes back (the real store emits `Changed` for them too), so a page hears
  one `privacy.changed` from `set_permission` and one from the store; both re-reads give the stored value. The
  stand-in in settings-privacy.py now behaves the same way.
* The watch thread wakes once a second to read its stop flag (and at once on `shutdown`); it exists only while the
  privacy service does (the first `privacy.*` call of a host starts it).

### Runs (16/17-11, worktree f, on 640729ae + these fixes)

* `wt/build.sh f all`: host and native release builds ok; native-release refreshed (19 binaries, 11 updated, among
  them `win11-gtk-settings` and the host binaries).
* `wt/host-tests.sh f --lib` (full): 605 passed, 1 failed, 19 ignored. The failure is still the known
  `services::update::elf::tests::our_binaries_equal_readelf` (`libm.so.6`, slice 1's note 7, not F's). New: 5 host
  tests (`the_sandbox_guard_needs_all_of_run_tests_environment`, `location_accuracy_is_read_only_as_gnome_settings_has_no_control`,
  `the_security_centre_is_named_in_en_gb`, `a_change_in_the_permission_store_from_elsewhere_reaches_the_pages`, and
  the rewritten `the_extension_hears_the_sound_and_toast_settings`). No test printed "skipped": the stricter guard
  still sees the sandbox. A filtered run first (`privacy notifications apps::defaults background gsettings`): 60
  passed, 0 failed.
* `wt/native-tests.sh f`: 81 passed, 0 failed, 1 ignored (win11-gtk-settings 46, 5 of them new); no warnings in the
  settings crate. The first attempt did not compile (`device_key` returned a reference to a temporary `GKey`); it
  returns the key by value now.
* `wt/checks.sh f`: as before (rustfmt 0 failing; check_rust 0 errors, 57 warnings; drift 0 errors, 15 warnings;
  native 0 errors, 48 uncovered; test_native, settings-pages and release OK; installer 70 as at the base).
* Harness, through `wt/harness.sh` with `--strace`:

| run | checks | failed | known | strace connect / exec | clicks (all real) |
|---|---|---|---|---|---|
| settings-apps nl | 134 | 1 | 1 | ok (364) / ok (621) | 47 |
| settings-apps en | 145 | 1 | 1 | ok (364) / ok (637) | 47 |
| settings-privacy nl | 107 | 1 | 1 | ok (226) / ok (328) | 24 |
| settings-privacy en | 117 | 1 | 1 | ok (226) / ok (328) | 24 |

  The one failure per run is the known kit dialog check (settings-b note 9). Every new check passed in both
  languages. The first settings-apps en run had 5 more failures. One pointer click on WinPoint's Meldingen switch in
  the expander header did not flip it (the `off` wait timed out, and the second click turned `showToasts` off). The
  four sound checks then saw no toast, because toasts stayed off. That switch, its `pointer_flips` and the press
  helper are unchanged by these fixes, and nl passed. The rerun above (the committed `settings-apps.en.out`) passed
  them all. For I: this header switch can lose a pointer click now and then (harness step, note 2, has the kit-wide
  fix for a switch inside a button).
* The stand-in check of f.py's new `permissionStore` branch, run once on a throwaway `dbus-run-session` bus: Lookup
  gives `yes`, then SetPermission `no`, then Lookup gives `no`, with one `Changed ('devices', 'camera', false, '',
  {'org.gnome.Nautilus': ['no']})`; other tables unchanged.

## CHANGELOG lines for I (batch F)

For the next release's section of `CHANGELOG.md` (I's file), in its style:

- **Apps as in Windows 11.** Installed apps has Advanced options for every app: its version and size, its
  permissions (location, camera, microphone, screenshots, background, notifications, search), whether it runs at
  sign-in, its global shortcuts, what a Flatpak or snap may access, the file and link types it opens, and Terminate,
  Reset (clear its cache) and Uninstall. Open in App Center or GNOME Software shows the app in the store that has it.
- **Default apps by file type and link type.** Default apps lets you search a file or link type and choose its app,
  or pick an app and see everything it opens, as Windows does; GNOME's defaults per kind (browser, mail, calendar,
  calls, text messages, music, video, photos, files, text editor, terminal) are there too, and one button resets all.
- **Notifications per app.** System › Notifications lists every app with its own page: banners, their content, the
  lock screen and the sound, as GNOME stores them. A GNOME notification now plays its sound with WinPoint's toast,
  unless you turned sounds off.
- **Privacy & security.** New pages for Device security (firmware checks and Ubuntu's Security Centre), Diagnostics
  & feedback (error reports and Ubuntu's system reports), Activity history, Search permissions, Searching this PC
  (the indexed places), Location, Camera and Microphone. The app lists follow every change, also one you answer in
  a permission dialog while the page is open.
- **Background and colours.** Personalisation › Background offers GNOME's pictures and your own, with the fit
  choice; Colours shows the GNOME accent colour that matches the Windows one you pick.
