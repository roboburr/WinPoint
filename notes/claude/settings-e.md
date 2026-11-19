# Settings parity, batch E: Accounts, time & region, About

Batch E's handoff note (notes/claude/settings-parity.md §4.6, §5.6). The batch is built in two slices; each appends
its section here.

## Slice 1: Accounts, Je info, E-mail en accounts, Aanmeldingsopties, Andere gebruikers

### What was built

Native pages (native/apps/settings/src/pages/), each replacing B0's placeholder, in Windows 11's order and wording
(the strings are rows of tests/i18n/windows-glossary/e.tsv):

* **accounts** (`accounts.rs`): Windows' header from `users.self` (picture, name, "Lokaal account", Administrator /
  Standaardgebruiker; the shell's own user as before when AccountsService is not there), the group
  "Accountinstellingen" (`common::child_links`), "Nu vergrendelen" and the privacy line. The module also holds the
  helpers the other Accounts pages share (pub(super)): `account_type_text`, `display_name`, `user_hero`,
  `user_avatar`, `refresh_on` (a page is built again 300 ms after a burst of `users.changed` / `goa.changed`),
  `admin_locked` (lockdown `user-administration-disabled`, as GNOME's Users panel honours it) and `PasswordForm`
  (Huidig wachtwoord, Nieuw wachtwoord, Wachtwoord bevestigen, GNOME's strength bar and hint from
  `users.passwordQuality`, "Een wachtwoord genereren"; its button rule is GNOME's: typed twice alike, level above 1).
* **yourinfo** (`yourinfo.rs`): the large header; "Je foto aanpassen": "Een bestand kiezen · Bladeren door bestanden"
  (GTK's file dialog, the FileChooser portal), GNOME's gallery as the expander "Een standaardafbeelding kiezen"
  (`users.faces`), "Foto verwijderen" (`users.setIcon {path: ""}`); "Naam wijzigen" (WinPoint addition, GNOME's Name
  row, `users.setRealName`); related: Aanmeldingsopties, Andere gebruikers.
* **email-accounts** (`email_accounts.rs`): "Accounts die worden gebruikt door e-mail, agenda en contactpersonen" with
  "Een account toevoegen · Account toevoegen" (the provider list of `goa.providers`; `goa.add` starts GOA's own dialog
  in `win11-gtk-settings --goa-add <type>`), an expander per account (presentation identity, provider, "Aanmelden
  vereist" with GNOME's attention state and "Opnieuw aanmelden" = `goa.signIn`), "Accountinstellingen · Beheren" (a
  dialog with one switch per service the account's provider has, `goa.setService`) and "Dit account verwijderen ·
  Verwijderen" (Windows' question, `goa.remove`; insensitive for an account the organisation manages, IsLocked);
  GNOME's offline banner (`system.network.get` connectivity "none"); related: Standaard-apps.
* **signin** (`signin.rs`): "Manieren om je aan te melden": Vingerafdrukherkenning (only with a reader:
  `caps.fingerprint` and `users.fingerprint.get`; Instellen / Nog een vinger toevoegen with the finger choice, the
  stages from `users.fingerprint.progress`, Vingerafdrukken verwijderen) and Wachtwoord ("Wijzigen": "Je wachtwoord
  wijzigen", `users.changePassword`). "Aanvullende instellingen": amendment 23's question "Als je weg bent geweest,
  wanneer moet WinPoint vereisen dat je je opnieuw aanmeldt?" (Wanneer het scherm wordt uitgeschakeld, Na 30
  seconden ... Na 1 uur, Nooit; one `gsettings.set` of `lock-enabled` + `lock-delay`, a non-preset delay stays as an
  option of its own, named `st-signin-lock-delay`), under it the checkbox "Ook vergrendelen wanneer de pc in slaapstand
  gaat" (`ubuntu-lock-on-suspend`), "Nieuwe USB-apparaten blokkeren wanneer de pc is vergrendeld" (`usb-protection`,
  only with `caps.usbguard`, as GNOME) and "Automatisch aanmelden" (WinPoint addition, `users.setAutoLogin`). Lockdown
  `disable-lock-screen` makes the lock rows insensitive. Related: Energie en batterij, Andere gebruikers.
* **otherusers** (`otherusers.rs`): "Een andere gebruiker toevoegen · Account toevoegen" ("Een gebruiker voor deze pc
  maken": Volledige naam, Gebruikersnaam suggested from it as GNOME does, Accounttype, "De gebruiker stelt een
  wachtwoord in bij de eerste aanmelding" / "Nu een wachtwoord instellen" with the password form; `users.create`), per
  user an expander (picture, name, "Administrator · Lokaal account") with "Accountopties · Accounttype wijzigen"
  (Windows' dialog), GNOME's "Wachtwoord opnieuw instellen" (next sign-in or now, `users.setPassword`), "Automatisch
  aanmelden" and "Taal" (see below), and "Account en gegevens · Verwijderen" ("Account en gegevens verwijderen?" with
  GNOME's Bestanden verwijderen / Bestanden bewaren, `users.delete`; a user who is still signed in gets GNOME's
  "{name} is nog aangemeld" with only Annuleren, review finding 1). The buttons that ask for the administrator
  password carry the shield (`pickers::admin_button`); lockdown `user-administration-disabled` makes them insensitive
  with the organisation tooltip.
* **goa_helper.rs**: `--goa-add <provider>` and `--goa-refresh <account id>` (the latter reaches `run` as
  `refresh:<id>` through `requested`, so main.rs stays as B0 froze it): libgoa-1.0 and libgoa-backend-1.0 through
  dlopen, `goa_client_new_sync`, `goa_provider_get_for_provider_type`, `goa_provider_add_account` /
  `goa_provider_refresh_account` with no parent widget (GOA's dialog is a window of its own), a GLib main loop until
  the callback; exit 0 done, 1 cancelled or failed, 2 bad arguments / GOA missing / no display.

Host (§5.6):

* `host/src/services/users.rs` + `users/{accounts.rs, passwd.rs, crypt.rs, fprint.rs, quality.rs, tests.rs}`: the 17
  methods `users.self`, `.list`, `.faces`, `.setRealName`, `.setIcon` (a stock picture as it is, any other picture
  scaled to its middle square of at most 512 px in the host's cache, 0600, removed after SetIconFile), `.setLanguage`,
  `.setAccountType` (refuses to demote the last administrator), `.setAutoLogin`, `.create` (CreateUser, then
  SetPassword with a libxcrypt hash or SetPasswordMode 1), `.delete` (never the own account), `.setPassword` (`now` /
  `atLogin`), `.changePassword` (passwd on a pseudo-terminal of its own with LC_ALL=C; `Transcript` reads PAM's
  prompts; a refused new password is not retried: passwd is stopped where it asks again), `.passwordQuality`
  (libpwquality: level 0-5, GNOME's hint per error, or `{generate: true}`), `.fingerprint.get`, `.enroll`, `.cancel`,
  `.delete` (fprintd; a watcher thread follows EnrollStatus, emits `users.fingerprint.progress {stage, stages, done,
  result}` and releases the reader when done). Events `users.changed {}` after every write and, debounced, for any
  AccountsService signal. Every daemon write goes through `call_interactive`; under `--dry-run` the arguments are
  checked and nothing is called (tests).
* `host/src/services/goa.rs`: `goa.list` (ObjectManager; a service is offered when its interface is on the account or
  its `<Service>Disabled` is true), `goa.providers` (GOA 3.58's Google, Microsoft 365, Microsoft Exchange, Nextcloud,
  IMAP en SMTP, WebDAV, Zakelijke aanmelding (Kerberos); `canAdd` when libgoa-backend and win11-gtk-settings are
  installed, looked for as files so GTK never loads into the host), `goa.add`, `goa.setService`, `goa.signIn`
  (EnsureCredentials, else the helper's sign-in), `goa.remove`; event `goa.changed {}`.
* `host/src/api-methods/accounts.json` (secret: `users.changePassword`, `.create`, `.setPassword`,
  `.passwordQuality`), `host/i18n/users.tsv`, `host/i18n/goa.tsv`, tests/i18n/BASELINE-CHANGES.rust-settings-e.
* `host/src/services/gsettings/allow/accounts.rs`: `usb-protection-level` moved from Write to Read (GNOME has no
  control for it either, §6.1's rule); the rest of B0's rows stay.
* tests/dbus-fakes/fakes/e/accounts.py (AccountsService + fprintd on the private system bus) and goa.py (goa-daemon
  on the private session bus), used by the ignored tests `fakes_users_on_the_system_bus` and
  `fakes_goa_on_the_session_bus`.

### plan.json controls dropped or changed

* yourinfo "Een foto maken · Camera openen" (windows.json only): left out, GNOME Settings 50 on Ubuntu has no camera
  capture in its avatar chooser. GNOME's crop area for a chosen file is not offered: the middle square is used.
* email-accounts "Beheren": the service switches are in Windows' "Accountinstellingen voor ..." dialog, not loose
  in the expander. The provider list is plan.json's seven (GOA's Fedora provider is not offered, as plan.json).
* signin: Windows Hello's face, PIN and security key, "Dynamische vergrendeling", "Accountgegevens weergeven op het
  aanmeldingsscherm" and "Mijn opnieuw opstartbare apps automatisch opslaan" are not in plan.json and stay out.
  "Nooit" writes only `lock-enabled false` (GNOME keeps the delay); the fingerprint expander is named
  "Vingerafdrukherkenning" without "(Windows Hello)" (a rename row in e.tsv).
* otherusers "Taal": shown only once `machine.locales` answers (slice 2). The page expects `{locales: [{id, name}]}`
  (id a locale such as `nl_NL.UTF-8`, name its display name); slice 2: keep that shape or change otherusers.rs with
  it. Windows' kiosk is left out. Windows always deletes the files of a removed account; GNOME's choice (Bestanden
  verwijderen / Bestanden bewaren) is kept, with the warning that it cannot be undone.
* `users.setFormats` (formats page) and every `machine.*` method are slice 2's.

### For slice 2 (time, language, formats, language-admin, about, the rename dialog)

* `users.setFormats` belongs in host/src/services/users.rs: a METHODS row, the dispatch arm (a `set_user(args, true,
  "SetFormatsLocale", ..)` like `set_language`, with `accounts::valid_locale`), its api-methods/accounts.json entry.
* users/tests.rs has `FakeBus` (a fake daemon on the sandbox's private session bus: objects from an XML, method
  replies, properties, every call recorded with its interactive flag), `context(dry)` and `fake_calls(var)`
  (tests/dbus-fakes); goa.rs's tests use them too. machine/*'s tests can use the same.
* otherusers.rs shows the "Taal" row once `machine.locales` answers `{locales: [{id, name}]}`.
* tests/gnome/scenarios/settings-time.py is still to be written; settings-accounts.py shows how a scenario loads
  lib/standins/e.py by path and clicks a kit dialog's button.

### Tests (slice 1)

* Host, tests/rust-sandbox/run-tests.sh --lib: 590 passed, 1 failed, 21 ignored. The failure is note 6 below (B0's
  `powf` makes win11-app need libm.so.6; every fresh host build shows it). Batch E's 17 tests (and 2 ignored fake runs, below) in
  `services::users::tests` and `services::goa::tests`: the AccountsService and fprintd calls against fakes on the
  sandbox's private session bus (arguments, object paths, ALLOW_INTERACTIVE_AUTHORIZATION on every write), the
  enrolment's EnrollStatus handling and release, every write's dry run (nothing reaches a daemon or passwd), the
  passwd transcripts and a passwd stand-in on a real pseudo-terminal (changed, wrong current password, a refused
  new one not retried), yescrypt hashes, libpwquality's levels and hints in both languages, the picture scaling,
  the name, user name and locale rules, GOA's service rule, ids and providers, goa-daemon missing.
* tests/dbus-fakes/run.sh --batch e (fakes/e/accounts.py on the private system bus, fakes/e/goa.py on the session
  bus): `fakes_users_on_the_system_bus` and `fakes_goa_on_the_session_bus` passed (2/2).
* Native, cargo test --release: 68 passed, 1 ignored (win11-gtk-settings 33, 9 of them batch E's: the password rule,
  the lock question's two keys and options, the enrolment texts and the host's finger names, the user name
  suggestion, GOA's service names against the host's table, the helper's command line and GOA's symbols).
* GNOME harness, tests/gnome/scenarios/settings-accounts.py (AccountsService stand-in, lib/standins/e.py): nl 16/16
  checks passed, en 21/21 (the same plus no Dutch word on each of the five pages): the pages are built, the header
  names the stand-in's account, Naam wijzigen sends SetRealName, the lock-on-suspend checkbox writes
  `ubuntu-lock-on-suspend`, the lock question follows `lock-enabled` and `lock-delay` written from outside (Na 5
  minuten, Nooit), both Automatisch aanmelden switches send SetAutomaticLogin, every write with interactive
  authorization; E-mail en accounts without goa-daemon offers no add and Aanmeldingsopties no fingerprint row. The
  kit dialog's button is clicked through the dev channel (as native-gtk-apps.py does): a real pointer click on it
  did not reach the button in this harness.
* tools: rustfmt, no-webkit, i18n (0 errors; the 57 rust and 48 native-dutch-only warnings are the base's),
  check_pairs, drift (15 warnings, the base's), test_native.py, settings-pages, release: green; the installer test
  refuses to run in this environment (XDG_RUNTIME_DIR not 0700), as before the batch.

### Notes for I

1. tools/i18n/glossary.tsv has "Naam wijzigen = Rename" (Verkenner's rename). For a person's name, Windows' Control
   Panel and GNOME say "Change name"; yourinfo.rs carries `// glossary-ok: Naam wijzigen` on its two places. A context
   row in tools/i18n/glossary-context.tsv (`native/apps/settings/src/pages/yourinfo.rs:*` Naam wijzigen → Change name)
   would replace the two markers.
2. The navigation pane's account header (app.rs, frozen) still reads `system.info`; it could show Windows' second
   line ("Lokaal account") and follow `users.changed` with `users.self`. Optional.
3. The harness redaction of secret methods (amendment 53) must cover `users.changePassword`, `users.create`,
   `users.setPassword` and `users.passwordQuality` (their args carry passwords).
4. tests/gnome/lib/standins/__init__.py (B0, frozen) documents `from lib.standins import Spec, start`, but the
   harness loads scenarios by path and keeps tests/gnome off sys.path (python -I, harness.py `load_lib`), so `lib`
   is not importable there. settings-accounts.py loads the package by path instead (`_load`, with `submodule_search_locations`, so
   `lib/standins/e.py`'s `from . import Spec` works). I: fix the docstring, or give scenario.py a `standins()` helper
   that does the same, and let the batches' scenarios use it.
5. host/src/services/goa.rs's "Microsoft Exchange" is a brand name (GOA's provider): tests/i18n/allow-host-settings-e.txt
   lists it for drift_rust.py. The translated provider names are catalogue rows (goa.provider.*).
6. `services::update::elf::tests::our_binaries_equal_readelf` fails on every new host build, not only E's (batches
   A and B's win11-app show the same): B0's `settings_mirror.rs` linearises sRGB with `powf(2.4)`, so win11-app now
   needs `libm.so.6` (`pow@GLIBC_2.29`), which tests/update/fixtures/compat/readelf-needed.txt does not list. I: add
   `libm.so.6` to the win11-app row (before `libc.so.6`, readelf's order) or compute the curve without `powf`.

## Slice 2: Systeem, Info, Tijd en taal, Datum en tijd, Taal en regio, Regionale notatie, Beheerinstellingen voor taal

### What was built

Native pages (native/apps/settings/src/pages/), each in Windows 11's order and wording (the strings are rows of
tests/i18n/windows-glossary/e.tsv, section "slice 2"):

* **system** (`system.rs`): the device card keeps `device_hero`'s signature; it shows the pretty name
  (`pc_name`: system.info `prettyHostname`, else `hostname`) and rebuilds on `machine.changed {what: name}`.
  `rename_hint(p, name)` (signature kept, home.rs and about.rs call it) is now Windows' dialog "De naam van je pc
  wijzigen": "Huidige pc-naam: …", the entry (`st-system-rename-entry`, 64 characters as GNOME's entry), Windows' hint
  "Je kunt een combinatie van letters, koppeltekens en cijfers gebruiken.", "Volgende" → `machine.setName`. Windows'
  restart step is left out (Linux needs none).
* **about** (`about.rs`): Windows 25H2's About: the device with "Naam van deze pc wijzigen" (insensitive without
  hostname1), the four cards (Processor, Geïnstalleerd RAM, Opslag, Grafische kaart), "Apparaatspecificaties" with
  Kopiëren (Apparaatnaam, Model = GNOME's Hardware Model, Processor, Geïnstalleerd RAM with "(… bruikbaar)", Grafische
  kaart from switcheroo-control, Opslag = GNOME's disk capacity from UDisks2 (else the sum of system.info's internal
  disks; the card shows the total, the row the total and the drives), Firmwareversie, Systeemtype "64-bits
  besturingssysteem, x64-processor") and "{os}-specificaties" with Kopiëren (Editie, Versie, Geïnstalleerd op, Build
  van besturingssysteem, GNOME-versie from GNOME Shell's ShellVersion (amendment 37), Venstersysteem, Virtualisatie,
  Kernel, Ervaring = WinPoint's version, as windows.json suggests). Related: Opslag, Prestaties bekijken, Updates.
  Rows without a value are left out, as GNOME hides them.
* **time** (`time.rs`, three pages):
  * Tijd en taal: the live clock with "Tijdzone: (UTC+01:00) Amsterdam" and WinPoint's language, the child links.
  * Datum en tijd, amendment 16's order: Tijd automatisch instellen (`machine.time.setNtp`, `st-datetime-ntp`),
    Tijdzone automatisch instellen (automatic-timezone, shown where caps.geoclue, insensitive with "Hiervoor moeten
    locatievoorzieningen zijn ingeschakeld." while org.gnome.system.location enabled is false), De datum en tijd
    handmatig instellen · Wijzigen (only while NTP is off; the dialog "Datum en tijd wijzigen" with the B0 Date and
    Time pickers → `machine.time.setTime`), Tijdzone (`machine.time.zones`, insensitive while the automatic zone is
    on and location services are on: GNOME's rule, review finding 6; `st-datetime-timezone`), Automatisch aanpassen aan zomertijd (on and insensitive), the tray switches
    (clock-show-date, -seconds, -weekday, calendar show-weekdate, all Mirrored rows), Aanvullende instellingen › Nu
    synchroniseren (`machine.time.sync`, with "Laatste geslaagde tijdsynchronisatie" once the host saw
    NTPSynchronized turn true, or the synchronised state, and "Tijdserver: …" from chrony's files), related links to
    Regionale notatie ("24-uursnotatie, eerste dag van de week …", amendment 16) and Taal en regio. The page follows
    `machine.changed {what: time}`.
  * Taal en regio (amendments 17, 18, 59): "Taal": Weergavetaal (`st-language-display`: the generated locales whose
    language support is installed, by their endonyms, plus the account's current one; `users.setLanguage`) and "Taal
    van WinPoint" (settings.language "system" = "Weergavetaal gebruiken (aanbevolen)", "nl", "en"; the widget keeps
    the name `language-display`, which tests/gnome/lib/scenario.py `set_language(via='dropdown')` opens); Windows'
    sign-out banner ("WinPoint moet je afmelden om je nieuwe weergavetaal toe te passen." · Afmelden =
    system.power.action logout) while the account's language differs from the session's. "Voorkeurstalen": "Een taal
    toevoegen" (shield; the dialog "Een taal kiezen om te installeren" with a search box → `machine.languages.install`),
    a progress card from `machine.changed {what: languages, percent}`, each installed language with "Taalpakket
    geïnstalleerd" or GNOME's "not completely installed" check with Installeren (`complete`: its language packs are
    there; the dictionaries and fonts pkg_depends names may not exist in the archive, PackageKit's Resolve sorts
    those out), and ⋯ › Taalopties. "Regio":
    Regionale notatie as an expander (the drop-down; Agenda, Eerste dag van de week, Korte datum, Korte tijd, Lange
    tijd, Getallen; Notaties wijzigen → formats). Related: Beheerinstellingen voor taal, Typen.
* **formats** (`formats.rs`): Regionale notatie (region key + `users.setFormats`, "Aanbevolen [Nederlands
  (Nederland)]" = "" with the display language's locale as FormatsLocale, review finding 7) with GNOME's preview (Datums %x, Tijden %X, Datums en tijden %c, Getallen, Maatsysteem,
  Papier), and "Notaties wijzigen": Agenda (Gregoriaanse kalender), Eerste dag van de week (week-start-day,
  `st-formats-week-start-day`), Korte datum, Korte tijd (clock-format and, as GNOME, both file choosers'
  clock-format in one `gsettings.set {writes}`; `st-formats-interface-clock-format`), Lange tijd. The sign-out banner
  when the formats differ from the session's LC_TIME. Shared helpers for time.rs: `region_dropdown`,
  `preview_rows`, `spec_rows`, `signout_banner`, `week_start_*`, `time_sample`.
* **language-admin** (`language_admin.rs`): "Welkomstscherm en nieuwe gebruikersaccounts": Taal van het
  aanmeldingsscherm and Notaties van het aanmeldingsscherm (`machine.login.setLocale {locale}` / `{formats}`,
  "Hetzelfde als de taal" = ""), "Mijn instellingen kopiëren · Instellingen kopiëren" (shield) with the own display
  language and regional format. Follows `machine.changed {what: login}`.

Language resolution (amendment 18): settings.language takes "system" (host/src/settings.rs `valid_language`, tests in
`mod language_tests` after `mod tests`), which reads as the session's messages language as gettext picks it (LC_ALL,
LC_MESSAGES, LANG, LANGUAGE's first entry unless the locale is C; Dutch → nl, anything else → en): host/src/i18n.rs
`Lang::of` + `system_code` (+ `mod system_language_tests`), native/kit/src/i18n.rs `resolve` + `system_language`,
native/shell/src/core.rs `Core::lang()` + `system_lang` (a test in its `mod tests`), native/calendar/src/config.rs's
`en` line + `system_is_english` (a private helper next to `load()`).

Host (§5.6), host/src/services/machine.rs + machine/{info, hostname, time, zones, locale, languages, tests}.rs:

* `machine.info` (hostname1 GetAll with /etc and DMI fallbacks, os-release, org.gnome.Shell ShellVersion, systemd1
  Virtualization with GNOME's labels, switcheroo GPUs default first, UDisks2 non-removable non-ejectable drive sizes,
  XDG_SESSION_TYPE, uname).
* `machine.setName` (GNOME's `pretty_hostname_to_static` through GLib's `g_str_to_ascii`; SetPrettyHostname then
  SetStaticHostname, each `(name, true)` through call_interactive).
* `machine.time.get` / `.setNtp` / `.setTime` (refused while NTP is on, 2000-2199) / `.setTimezone` (a tzdata zone) /
  `.sync` (SetNTP false, true; refused while NTP is off; the second call retried once so NTP never stays off) /
  `.zones` (zone1970.tab + UTC + the current zone, Windows' "(UTC±hh:mm) City" with the standard offset = the smaller
  of January and July). No timesync1 anywhere (amendment 30). The server: chrony.conf `server`/`pool`/`peer` lines and
  its `confdir`/`sourcedir` files, else timesyncd.conf `NTP=`/`FallbackNTP=`.
* `machine.locales {preview?}` (localedef --list-archive through system_tool; else glibc's locale archive read as
  gnome-desktop reads it, plus the locale directories; names from iso-codes' JSON and .mo catalogues, endonyms in the
  locale's own language; `supported` = its language pack is installed; the session's LANG/LC_TIME; GNOME's Formats
  preview through newlocale/strftime_l/nl_langinfo_l, with the locale's grouping, measurement and paper size).
* `machine.login.get` / `.setLocale {locale?, formats?}` (locale1; GNOME's list: LANG and, with formats, the five LC_
  entries).
* `machine.languages` / `.languages.install {code}` (language-selector's languagelist, locale2langpack and pkg_depends,
  dpkg's status file read only; PackageKit Resolve (not installed, newest) then InstallPackages (only trusted) with
  ALLOW_INTERACTIVE_AUTHORIZATION and the `interactive=true` hint, on a thread of its own; progress and the end as
  `machine.changed`; one install at a time).
* A watcher thread follows hostname1, timedate1 and locale1 PropertiesChanged → `machine.changed {what}`.
* `users.setFormats` (users.rs: SetFormatsLocale, `valid_locale`, interactive).
* host/src/services/system/info.rs: `system.info` reads hostname and prettyHostname fresh on every call (the 120 s
  cache would show the old name after a rename).
* api-methods/accounts.json (13 machine methods + users.setFormats), host/i18n/machine.tsv (16 rows),
  BASELINE-CHANGES.rust-settings-e, allow/accounts.rs: `org.gtk.Settings.FileChooser` and
  `org.gtk.gtk4.Settings.FileChooser` `clock-format` (Write; GNOME's datetime panel writes them too).
* tests/dbus-fakes/fakes/e/machine.py (hostname1, timedate1 without timesync1, locale1, PackageKit with Package and
  Finished signals); accounts.py and lib/standins/e.py got SetFormatsLocale/FormatsLocale; lib/standins/e.py got
  `timedate1()`, `hostname1()`, `locale1()`; tests/gnome/scenarios/settings-time.py.

### plan.json controls dropped or changed

* datetime: the 24-hour row and the first day moved to formats (amendment 16); the old "Datum weergeven op de
  taakbalk" and "Seconden weergeven" WinPoint rows are now GNOME's tray keys (mirrored). Windows' "Aanvullende
  agenda's weergeven op de taakbalk" and the related "Extra klokken" (org.gnome.shell world-clocks) are not in
  plan.json and stay out. "Laatste geslaagde tijdsynchronisatie" is the moment this host saw NTPSynchronized turn
  true (chrony has no D-Bus status; amendment 30 rules out timesync1 and chronyc was not added).
* Time zone labels are IANA's city names ("(UTC+01:00) Amsterdam"), not Windows' grouped CLDR lists ("Amsterdam,
  Berlijn, Bern, Rome, Stockholm, Wenen") and not translated (WinPoint ships no city table): a city search like
  GNOME's is not offered either, the drop-down lists the ~310 zones of zone1970.tab.
* formats: Windows' "Lange datum" has no POSIX counterpart (a locale defines %x, %X and %c only): the preview shows
  GNOME's "Datums en tijden" (%c) instead. "Lange tijd" shows the clock format's sample, not the locale's %X.
  Windows' "Land of regio" has no GNOME function and is left out.
* language: AccountsService keeps one language, so "Voorkeurstalen" has no Omhoog/Omlaag; it lists the installed
  languages (English always). Removing a language (Windows' ⋯ › Verwijderen) is not offered: GNOME has no such
  function and removing language packs is apt's job.
* about: the old "Actief sinds opstarten", "Gebruiker" and "Bureaublad" rows are gone (not Windows' or GNOME's
  About rows: Taakbeheer and Accounts show them; Venstersysteem and GNOME-versie replace Bureaublad). Apparaat-id
  and Product-id are left out (windows.json: privacy, no counterpart).
* language-admin: Windows' "Landinstelling van systeem wijzigen" (non-Unicode programs) has no counterpart; the
  sign-in screen's keyboard (locale1 SetX11Keyboard) is the keyboard pages' (batch A).

### Tests (slice 2)

* Host, tests/rust-sandbox/run-tests.sh --lib: 605 passed, 1 failed, 22 ignored; the failure is note 6
  (`our_binaries_equal_readelf`, B0's powf), as in slice 1. New: `services::machine::tests` (11, plus the ignored
  system-bus run): the static hostname (GLib's transliteration, GNOME's dashes and 63 characters), zone labels and
  standard offsets (Amsterdam +01:00 in summer too, Kolkata +05:30, St. John's -03:30, Dublin +00:00) and the
  list's order, chrony's `server`/`pool` lines with `confdir`/`sourcedir` and timesyncd's `NTP=`, locale
  normalisation and the UTF-8 rule, the locale archive and .mo readers on synthesised files, iso-codes' names and
  endonyms ("Nederlands (Nederland)", "Engels (Verenigde Staten)"), the preview's grouping (en_IN's 3;2) and the
  en_GB/en_US measurement and paper, locale1's list, language-selector's rules against a dpkg fixture; on fakes on
  the private session bus: every write's D-Bus arguments with ALLOW_INTERACTIVE_AUTHORIZATION (SetPrettyHostname,
  SetStaticHostname, SetNTP false/true for Nu synchroniseren and nothing else, SetTime refused while NTP is on,
  SetTimezone, SetLocale), the dry runs (nothing reaches a daemon), PackageKit's Resolve then InstallPackages of what
  it reported (the interactive hint and flag), one install at a time. Also `settings::language_tests` (2),
  `i18n::system_language_tests` (2) and `users.setFormats` in the users tests.
* tests/dbus-fakes/run.sh --batch e (accounts.py, goa.py, machine.py): `fakes_machine_on_the_system_bus`,
  `fakes_users_on_the_system_bus`, `fakes_goa_on_the_session_bus`: 3/3 passed (the test binary is copied into
  tests/dbus-fakes/.run/bin first: the layer only shows the worktree).
* Native, cargo test --release: 79 passed, 1 ignored (win11-gtk-settings 42: the rename rule and pc name, the
  system type and RAM lines, the manual time's microseconds, Taal van WinPoint's values, the display languages
  (installed ones only), Nu synchroniseren's lines and the zone options, the region options and effective region,
  the week-start values, the preview rows, the sign-in screen's options and "Mijn instellingen kopiëren"; kit:
  `resolve("system")`; shell: `system_lang`).
* GNOME harness, tests/gnome/scenarios/settings-time.py (stand-ins timedate1, hostname1, locale1, AccountsService):
  nl 26/26, en 33/33 (the same plus no Dutch word on the seven pages). Amendment 68: Datum en tijd shows no toast and
  Nu synchroniseren sends exactly SetNTP(false, true), SetNTP(true, true), both interactive; Seconden weergeven
  writes clock-show-seconds; the weekday switch follows clock-show-weekday written from outside; Korte tijd 12-uurs
  writes clock-format and the file chooser's copy; Instellingen kopiëren sends SetLocale(['LANG=nl_NL.UTF-8'], true);
  Info names hostname1's "WinPoint Test" and shows GNOME's version; the rename dialog sends ('Robins Laptop', true)
  and ('robins-laptop', true), interactive. The harness has no /var/lib/dpkg/status, so its Taal en regio shot shows English
  "not completely installed"; the last host change (English needs no language pack) came after these runs and only
  changes that line (unit test `language_support_follows_language_selectors_data`).
* tools: rustfmt, no-webkit, i18n (0 errors; warnings at the base 57 rust and 48 native-dutch-only), check_pairs,
  drift (15 warnings, the base), test_native.py, settings-pages, release: green; the installer test refuses to run
  here (XDG_RUNTIME_DIR not 0700), as before. Builds: host 28 lib warnings (the base, none in batch E files),
  win11-gtk-settings 0 warnings, win11-shell 19 (the base of every batch).

### Notes for I

7. `extension/extension.js` (the extension's own C1 reader, line ~46 `JSON.parse(text)?.language === 'en'`) and
   `host/src/files/mod.rs` `context()` (its external-settings branch: `match settings["language"].as_str()`) still
   read "system" as Dutch. Patch: in extension.js, `const l = JSON.parse(text)?.language; return l === 'en' ||
   (l === 'system' && !GLib.get_language_names()[0].startsWith('nl')) ? 'en' : 'nl';` (GLib's list is gettext's
   order); in files/mod.rs, `let language = crate::i18n::Lang::of(&settings["language"]).code().to_owned();`.
8. Amendment 18 says WinPoint follows the display language "by default": host/src/settings-defaults.json (frozen)
   still has `"language": "nl"`. Optional: make the default `"system"` (new installs follow the account's language;
   existing settings.json files keep their nl/en).
9. registry.rs (frozen) keywords: `datetime` could add "ntp", "synchroniseren"/"sync", "zomertijd"/"daylight";
   `language` "weergavetaal"/"display language", "taalpakket"/"language pack"; `language-admin` "aanmeldingsscherm"/
   "sign-in screen", "welkomstscherm"/"welcome screen"; `about` "hostname", "pc-naam"/"pc name", "gnome",
   "firmware", "wayland".
10. The harness redaction (note 3) needs nothing new: no machine method is secret.
11. `tests/gnome/lib/scenario.py` `_dropdown` finds the WinPoint language options by labels starting with
    "Nederlands"/"English": Taal van WinPoint keeps those labels, and its third option "Weergavetaal gebruiken" does
    not start with either.

## Harness step: settings-accounts.py and settings-time.py in the private GNOME

### What the scenarios check

Both follow plan.json "testing" (GNOME HARNESS) and tests/gnome/README.md. Per page: `h.open('settings:<id>')`, a
screenshot `<id>.<lang>` (PNGs gitignored), the title (top-level pages) or the breadcrumb (the parent chain's
`crumb-<id>` buttons and the current label, e.g. "Tijd en taal › Taal en regio › Regionale notatie"), no B0
placeholder, and in en no Dutch word. Every bound control of the pages is flipped by its `st-<page>-<key>` name with
real pointer clicks (the page is scrolled with the real wheel first; the fixed header covers its top 140 px), the
private keyfile is read with `h.gsettings_get` for GNOME's exact value, and the key is then written from outside with
`h.gsettings_set` and the control must follow:

* signin: `st-signin-lock-delay` (Na 5 minuten → lock-enabled true + lock-delay 300; Nooit → lock-enabled false, the
  delay kept; follows both keys), `st-signin-ubuntu-lock-on-suspend`.
* datetime: `st-datetime-clock-show-date`, `-clock-show-seconds`, `-clock-show-weekday`, `-show-weekdate`,
  `-automatic-timezone` (which waits for `org.gnome.system.location enabled` and takes the time zone drop-down away).
* formats: `st-formats-region` (region, plus AccountsService FormatsLocale), `st-formats-week-start-day`,
  `st-formats-interface-clock-format` (clock-format and both file-chooser copies in one write; Lange tijd follows).

No key of batch E's pages is WinPoint-owned (allow/accounts.rs has no Owned row), so there is no pointer to check;
locks are the host unit tests' (contract §8 note 8). Every daemon write must reach its stand-in with
ALLOW_INTERACTIVE_AUTHORIZATION (the polkit path, §5.8): SetRealName, SetIconFile (a stock picture, then ''),
SetAutomaticLogin (own and another user, each on its own object), SetAccountType, SetPasswordMode (reset and a new
user), CreateUser, DeleteUser; Account.Remove; fprintd Claim, EnrollStart, DeleteEnrolledFingers2; SetNTP (Nu
synchroniseren: exactly false then true, amendment 68; the switch), SetTime, SetTimezone; SetPrettyHostname and
SetStaticHostname; SetLocale (the sign-in screen and Instellingen kopiëren); SetLanguage; PackageKit's
InstallPackages (its Resolve is not interactive, its hints say so). Hardware and services: Aanmeldingsopties has no
fingerprint row without fprintd and shows it, with an enrolment that follows fprintd's EnrollStatus, once fprintd is
there (Settings started again: caps.get asks fprintd once per backend); no USB row without usbguard; E-mail en
accounts offers nothing to add without goa-daemon. Helpers: Wachtwoord wijzigen runs `passwd` (the run/stubs passwd,
logged in guard-calls.log, under `--strace` for settings-accounts.py), a GOA provider starts `win11-gtk-settings
--goa-add <provider>`. Mirror (plan "Mirror checks"): clock-show-seconds and clock-show-weekday reach WinPoint's
time.showSeconds / taskbar.showSeconds / time.showWeekday, the taskbar clock shows the seconds and the 12-hour clock.
Last, settings-time.py switches Taal van WinPoint with the real drop-down to the other language and back.

### Batch E's stand-in server (tests/gnome/lib/standins/e.py)

B0's builder (lib/standins/__init__.py, frozen) answers from fixed JSON, so a page that reads its daemon again after
a write saw the old value, and no daemon sent a signal. e.py has `start_live(h, spec, tag)`: the same Spec and call log
(`{path, iface, method, args, interactive}`), with GVariant text for the values, methods that change properties
(`set`, then PropertiesChanged), replies by argument (`replies`) or from state (`replyProp`), and signals after the
reply (`emit`, with a delay). The specs: AccountsService (alex, sam and kim, whom CreateUser lists and DeleteUser
takes away; each Set* sends Changed), fprintd (a press reader with one finger; EnrollStart passes its stages and
completes), goa-daemon (one Google account; Remove sends InterfacesRemoved), timedated, hostnamed, localed (their
Set* change the properties) and PackageKit (Resolve reports a package, InstallPackages 40 %, 100 %, Finished). The
account's language is en_US.UTF-8, the session's (the harness, as the live GNOME, runs LANG=en_US.UTF-8, plan D-G3;
the sandbox's locale archive is the host's, with English locales only). The file also holds the scenarios' helpers
(the page check, real-wheel scrolling, drop-downs whose option the pointer can reach, expanders by their
w11-expanded state, dialog buttons).

### Fixed in batch E's files because of the harness

* host users.rs, goa.rs, machine.rs (the watchers): the debounce of a daemon's signals used
  `glib::timeout_add_local_once` on the watcher thread. That attaches to the default MainContext, which the host's
  main thread owns, so it panicked, and a panic in a signal callback aborts: the first AccountsService Changed (a
  rename, an automatic sign-in), any GOA or hostnamed/timedated/localed property change stopped Settings' backend
  ("Geen verbinding met Win11 Shell" in the dialogs, the following writes lost). B0's frozen stand-ins never sent a
  signal, so slice 1 and 2 did not see it. Now `users::debounce` waits on a plain thread; unit test
  `a_burst_of_signals_is_one_event_off_the_default_main_context`.
* host machine/languages.rs: Ubuntu's languagelist line for Dzongkha has its fields in another order; it was the
  language "dz_BT" (shown as that code in Een taal toevoegen). The unit test's fixture has the line now.
* yourinfo.rs: GNOME's gallery drew its 64 px pictures at 128 px (common::avatar's gtk::Picture asks for its texture's
  size); `face_tile` is a gtk::Image with a pixel size.
* otherusers.rs: the dialogs' radio choices are named (`st-otherusers-type-choice-<type>`,
  `st-otherusers-add-mode-<mode>`, `st-otherusers-password-mode-<mode>`), so the harness can pick one; the password
  fields of Account toevoegen and Wachtwoord opnieuw instellen show only with "Nu een wachtwoord instellen", as in
  GNOME's dialog (the add dialog reached the window's bottom edge). accounts.rs lost `PasswordForm::set_sensitive`.
* formats.rs: Regionale notatie (and the region on Taal en regio) is built again when the region key changes, so
  the sign-out banner and the preview follow at once, as GNOME's Region panel shows its banner.
* goa_helper.rs: GOA's dialog window took the prgname `win11-gtk-settings`, so the shell showed a separate "GTK
  Application" in the taskbar and its "is ready" banner; it now has Settings' app id
  (io.github.win11shell.Settings), as GNOME Settings shows GOA's dialog in its own process.

### Runs

Built first (`build.sh e all`: host 28 lib warnings, the base; win11-gtk-settings 0), then each scenario in nl and in
en through the private GNOME (`harness.sh e`), settings-accounts.py under `--strace`. Results (results.json and the
logs; settings-accounts' strace.txt with its .json reports) are committed; the PNGs stay local (gitignored).

| Run | Checks | Failed |
|---|---|---|
| settings-accounts.py nl (`settings-accounts.out`, `--strace`) | 64/67 | 3, all known: `kit-dialog-gesture`, `harness-stub-dir-mode`, `sandbox-cracklib` |
| settings-accounts.py en (`settings-accounts.en.out`, `--strace`) | 71/74 | the same 3 known |
| settings-time.py nl (`settings-time.out`) | 93/94 | 1 known: `I-note-2` |
| settings-time.py en (`settings-time.en.out`) | 100/101 | 1 known: `I-note-2` |
| native-shell.py nl (regression, out dir not committed) | 21/21 | 0 |
| lock-unlock.py nl (regression, out dir not committed) | 5/5 | 0 |

en adds one "no Dutch word" check per page (and one per extra page state); none failed. The strace checks of both
settings-accounts runs pass (connect: 249 and 253 connect() calls, all inside the sandbox; exec: 523 execve() calls,
no violation): the one passwd that ran is run/stubs/passwd (no arguments; its line in guard-calls.log), no real
pkexec, passwd or apt. Unit tests after the fixes: host `services::users` 14 passed, 1 ignored (the new
`a_burst_of_signals_is_one_event_off_the_default_main_context` among them) and `services::machine` 11 passed, 1
ignored (the Dzongkha line); native cargo test --release 79 passed, 1 ignored (win11-gtk-settings 42).

### Known in the runs, and what the harness cannot show

* `kit-dialog-gesture` (known check): a real pointer click on a kit dialog's button (Annuleren of Naam wijzigen) does
  not reach it, nor one on a switch in a dialog (Beheren's Agenda); pointer clicks on page widgets and popovers do.
  The dialogs' buttons, switches and radio choices are clicked through the dev channel (native-gtk-apps.py does the
  same). Cause and patch: note 12.
* `I-note-2` (known check): the taskbar clock does not show the weekday yet; the mirror sets time.showWeekday
  (contract §8 note 2, native/shell/src/panel/taskbar/system.rs, I's).
* `sandbox-cracklib` (known check, settings-accounts.py): libpwquality's dictionary is /var/cache/cracklib, and the
  sandbox's /var is its own, so every password fails the dictionary check ("Vermijd veelgebruikte woorden", as GNOME
  Settings would show) and Wijzigen stays off. The scenario then makes the dialog's call through the app's backend
  (`users.changePassword`): passwd runs (the stub, no arguments) and its refusal comes back as an error.
* The System page's device card reads `system.info`, whose pretty name is /etc/machine-info's (hostnamed writes that
  file; the stand-in cannot), so in the harness it keeps the sandbox's name after a rename; Info reads hostnamed and
  shows the new one. On a real system both follow.
* Not in the harness: the file portal (Bladeren door bestanden), a real polkit agent, GOA's own sign-in, a real
  reader, PackageKit's real transactions, usbguard (caps from /usr/share/dbus-1/system-services, absent on this host).

### Notes for I

12. native/kit/src/controls/dialog.rs (kit, not batch E's): the dialog's smoke layer has a GestureClick (`block`) in
    the bubble phase that claims every press. A press on a button inside the dialog bubbles up to it (GtkButton
    claims its sequence only on release), the smoke claims the sequence and GTK cancels the button's gesture: in the
    harness no pointer click reaches a dialog's button or switch, only keys and the dev channel do. Patch (one line,
    not tried in a build): `block.set_propagation_phase(gtk::PropagationPhase::Target);` after
    `let block = gtk::GestureClick::new();`, so only a press on the smoke itself (outside the dialog) is claimed and
    the dialog stays modal. Then lib/standins/e.py `dialog_button` can click with the pointer.
13. tests/gnome/lib/standins/__init__.py (B0, frozen) could take e.py's `start_live` server (GVariant values,
    state, signals) for every batch; B0's JSON values cannot express a{sv} replies (GOA, PackageKit) or a daemon
    that changes after a write.
14. native/kit/src/controls/toggles.rs `RadioGroup` names no button, so a harness cannot pick a choice in a dialog
    (where only the dev channel reaches, note 12); otherusers.rs names its choices itself (`name_choices`). The kit
    could name each button `<group's name>-<value>` when the group has a name.
15. native/kit/src/dev.rs `tree()` reports a widget's `visible` flag, so the children of a closed expander (a
    GtkRevealer keeps them visible) are found by `h.find` and "clicked" at a place that shows something else.
    `widget.is_child_visible()` up the chain, or `is_mapped()`, would leave them out; until then e.py's `expand`
    reads the expander's w11-expanded class.
16. tests/gnome/harness.py `prepare()` makes run/stubs with the umask's mode (0775 here), and host/src/tools.rs
    (B0) takes `$WIN11_SYSTEM_TOOL_DIR` only when nobody else can write it: so in every harness run each
    `system_tool` (passwd, localedef, grdctl, pkexec, ...) was refused, and no stub was ever reached (localedef's
    fallback and passwd's "unavailable" hid it). Patch: `os.chmod(RUN / 'stubs', 0o700)` after the loop that makes
    the run's subdirectories. settings-accounts.py does that chmod itself before its passwd step and records the
    mode (known check `harness-stub-dir-mode`); remove that once the harness does it.

### Open (batch E's own files, not done in this step)

* system.info (host/src/services/system/info.rs) takes the pretty name from /etc/machine-info, GNOME Settings
  from hostnamed's PrettyHostname; on a real system both agree after a rename (hostnamed writes the file). The
  System page could read hostnamed too (machine.info has it), at the cost of a D-Bus call on every system.info.
* A display language or region that is not generated shows its locale id ("nl_NL.UTF-8") in Weergavetaal and in
  "Aanbevolen [...]"; GNOME names any locale (gnome_get_language_from_locale). machine.locales could name the
  account's and the session's language even when the locale archive lacks them.
* Een taal toevoegen shows the Chinese entries as "Chinees (China) (汉语 (中国))"; Windows writes "Chinees
  (vereenvoudigd, China)".

## Review of batch E (16-11): the eleven findings

Each finding was read against the code first (to refute it); all eleven held, and each is fixed with a test.

| # | Finding | Verdict | Fix | Test |
|---|---|---|---|---|
| 1 | major: `users.delete` deletes a user who is still logged in | fixed | users/accounts.rs `logged_in` (logind ListSessions, each session's Class and State; a session counts as libaccountsservice's reload_systemd_sessions counts it: Class "user", State not "closing") and `session_counts`; users.rs `delete` refuses `denied users.loggedIn` (new users.tsv row, GNOME's text) after the uid checks and before the dry-run return; every user of an answer has `loggedIn`; otherusers.rs shows GNOME's "{name} is nog aangemeld" with only Annuleren before it asks about the files | host `a_user_who_is_still_signed_in_is_not_deleted` (logind fake: anna online, the greeter's session no login; real and dry run refused, no DeleteUser), `accountsservice_calls_ask_polkit_and_carry_gnomes_arguments` (a closing session is no login); harness settings-accounts.py (logind stand-in: sam has a session) |
| 2 | major: Resolve without PK_FILTER_ENUM_ARCH installs foreign-architecture packages | fixed | languages.rs `FILTER_NOT_INSTALLED_NEWEST_ARCH` = NOT_INSTALLED, NEWEST, ARCH (327688, gnome-software's NEWEST, ARCH); `run` keeps only ids of the native architecture or "all" (`native_package`; `native_arch` = dpkg's own Architecture in dpkg's status file, else the build's) | `a_language_is_resolved_then_installed_through_packagekit` (an i386 Package never reaches InstallPackages, filter uint64 327688), `packagekit_ids_and_errors_read_as_gnome_software_reads_them` |
| 3 | minor: a cancelled or refused PackageKit password dialog is "installFailed" with PackageKit's English | fixed | `Outcome.error` keeps the PkErrorEnum code; `error_failure`: NOT_AUTHORIZED (48) is `denied core.notAuthorized` (PackageKit cannot tell a dismissed dialog from a refusal: both are "Failed to obtain authentication."), TRANSACTION_CANCELLED (17) `machine.installStopped`, others `machine.installFailed` with the detail; the values checked against PackageKitGlib's typelib on this host | `a_dismissed_password_dialog_is_no_failed_install` (ErrorCode 48 + Finished failed on the fake: the done event carries core.notAuthorized's text), `packagekit_ids_and_errors_read_as_gnome_software_reads_them` |
| 4 | minor: `users.fingerprint.busy` is never returned (the error name is stripped, fprintd's text is translated) | fixed | fprint.rs `claim` calls with ALLOW_INTERACTIVE_AUTHORIZATION itself and `claim_error` tests `remote_error_name == net.reactivated.Fprint.Error.AlreadyInUse`, else `interactive_error`; no message matching | `a_reader_in_use_is_busy_in_any_language` (fake Claim: AlreadyInUse with a Dutch message; PermissionDenied and Internal keep their keys) |
| 5 | major: the raw AccountsService Language ("nl_NL", as Ubuntu stores it) compared with generated ids | fixed | machine/locale.rs `language_locale` (Ubuntu's language2locale: codeset dropped, the exact `ll_CC[@mod]`, for a bare language main-countries' locale, else the first of that language, never a guess for zh), `account_language`, `locale_data` (generated locales cached on the archive's mtime, main-countries); users.rs `complete`: `language` and `formatsLocale` are generated ids (raw in `languageRaw` / `formatsLocaleRaw`), the own account without a language has the session's (GNOME's Region page: setlocale); machine.locales' `session` is resolved the same way. time.rs, formats.rs, language_admin.rs and otherusers.rs now compare and look up machine.locales' ids, unchanged | `an_account_language_is_the_generated_locale_ubuntu_means`, the AccountsService test (anna's "nl_NL" is nl_NL.UTF-8, raw kept); harness: the stand-ins store "en_US" (Ubuntu's form), and Weergavetaal must still show English (United States), no sign-out banner, Instellingen kopiëren must send LANG=en_US.UTF-8 |
| 6 | major: the time zone can't be chosen with the automatic time zone on and location services off | fixed | time.rs `zone_sensitive(available, auto_tz, located)` = GNOME's `!auto_timezone \|\| !location_allowed`, applied from both `follow(AUTO_TZ)` and `follow(LOCATION)` (the never-updated Cell is gone); the switch itself stays insensitive while location is off, as GNOME's row | native `the_time_zone_can_be_chosen_unless_location_sets_it`; harness settings-time.py (location off with the automatic zone on: the drop-down can be used; on again: it can't) |
| 7 | minor: "Aanbevolen" sends FormatsLocale "" (save-to-pam-env refuses it) and hides the failure | fixed | formats.rs `formats_locale`: "" sends the display language's locale (the account's resolved language, else the session's); `p.run` shows a refusal and the region key goes back to the previous value | native `the_recommended_format_sends_the_display_languages_locale`; harness settings-time.py (Aanbevolen: region '' and SetFormatsLocale('en_US.UTF-8'), interactive) |
| 8 | minor: an enrolment that ends without a fingerprint leaves the dialog dead | fixed | signin.rs `enroll_step`: on done without enroll-completed the dialog is usable again (finger drop-down, the primary button relabelled "Opnieuw proberen", the bar reset); the progress listener is the dialog's own (`kit::ctx().on`, removed when the dialog closes), so the page's rebuild after users.changed no longer cuts it off | native `an_enrolment_that_ends_without_a_fingerprint_can_be_tried_again` |
| 9 | minor: caps-dependent rows read the caps snapshot of the first build (Null on a cold start) | fixed | accounts.rs `caps_now(p)`: the cached caps, else the page's own `caps.get`; signin.rs (fingerprint, usbguard) and time.rs (geoclue) decide from it | no unit test (a page-level await of `caps.get`); the harness opens Aanmeldingsopties in a freshly started Settings with the fprintd stand-in and finds the fingerprint row |
| 10 | minor: Opslag lists drives the total does not count | fixed | machine/info.rs `disk_drives` (the UDisks2 drives disk_capacity sums: Vendor and Model, Size), machine.info `disks`; about.rs `storage_drives` lists those (without UDisks: system.info's disks without USB for both the total and the list; a total without a list shows the total alone) | host `info_lists_the_drives_of_its_disk_capacity`, native `the_storage_list_adds_up_to_its_total`; the harness's about.en.png (this host's drives) now reads "25.7 TB: ST24000DM001-3Y7103 (21.8 TB), ST3000VX009-2AY10G (2.7 TB), Lexar SSD NM620 1TB (954 GB), GIGABYTE GP-GSM2NE8256GNTD (238 GB)", which adds up (the HGST and the Kingston, which UDisks counts as removable, are no longer listed) |
| 11 | minor: GOA's ticketing service named "Netwerkaanmelding (Kerberos)" | fixed | email_accounts.rs: "Netwerkbronnen" / "Network Resources" (GNOME Online Accounts' "Network _Resources", libgoa-backend-1.0 on this host); the e.tsv row follows (winpoint, verified=no) | the glossary checks; native `the_names_cover_the_hosts_services` (every host service still named) |

New UI strings (tests/i18n/windows-glossary/e.tsv): "{name} is nog aangemeld" / "{name} is still signed in" and GNOME's
body "Een gebruiker verwijderen terwijl die is aangemeld, kan het systeem in een inconsistente toestand achterlaten." /
"Deleting a user while they’re signed in can leave the system in an inconsistent state." (winpoint, no: GNOME's
"%s is still logged in" in Windows' aanmelden wording), "Opnieuw proberen" / "Try again" (glossary, yes), "Netwerkbronnen"
/ "Network Resources". Host: users.loggedIn (users.tsv, BASELINE-CHANGES.rust-settings-e). api-methods/accounts.json's
docs name the new fields (users.self/list `loggedIn`, `languageRaw`, `formatsLocaleRaw`; machine.info `disks`) and
users.delete's refusal.

Harness changes: lib/standins/e.py `STORED_LANGUAGE = 'en_US'` (Ubuntu's stored form; the users start with it, the
pages still show and send `LANGUAGE = 'en_US.UTF-8'`) and `logind(sessions)` (Manager.ListSessions, Session Class
"user" / State "online"), started only around settings-accounts.py's last otherusers step; settings-time.py: the
zone with location off, FormatsLocale for Aanbevolen, the language put back in the stored form.

### Runs (review)

* Host, tests/rust-sandbox/run-tests.sh --lib (full): 612 passed, 1 failed, 22 ignored; the failure is note 6
  (`our_binaries_equal_readelf`: win11-app needs libm.so.6 since B0's powf, unchanged). Batch E's modules:
  `services::users` 17 passed + 1 ignored (3 new), `services::machine` 14 + 1 (3 new), `services::goa` 4 + 1,
  `settings::language_tests` 2, `i18n::system_language_tests` 2.
* tests/dbus-fakes/run.sh --batch e (`--ignored fakes_`): 3/3 passed (users, machine, goa on the private buses; the
  users fake has no logind, so nobody counts as logged in there).
* Native, cargo test --release: 83 passed, 1 ignored (win11-gtk-settings 46: 4 new).
* Builds (build.sh e all): host 28 lib warnings (the base, none in batch E files), win11-gtk-settings 0, win11-shell
  19 (the base); tests/build/native-release refreshed (19 binaries, SHA256SUMS).
* checks.sh: rustfmt, no-webkit, i18n (0 errors, 57 rust warnings: the base), pairs, rust, drift (15 warnings, the
  base), native, test_native.py, settings-pages, release: green; installer refuses here (XDG_RUNTIME_DIR not 0700),
  as before.
* GNOME harness (results.json and logs committed, PNGs local):

| Run | Checks | Failed |
|---|---|---|
| settings-accounts.py nl (`settings-accounts.out`, `--strace`) | 76 | 3, all known: `kit-dialog-gesture`, `harness-stub-dir-mode`, `sandbox-cracklib` |
| settings-accounts.py en (`settings-accounts.en.out`, `--strace`) | 85 | the same 3 known |
| settings-time.py nl (`settings-time.out`) | 97 | 1 known: `I-note-2` |
| settings-time.py en (`settings-time.en.out`) | 104 | 1 known: `I-note-2` |

  The strace checks of both settings-accounts runs pass (connect 263 / 264 calls, exec 558 / 556, no violation).
  The new checks pass: the "is nog aangemeld" dialog with only Annuleren, users.delete's refusal and no DeleteUser
  (logind stand-in), the zone with location services off, FormatsLocale en_US.UTF-8 for Aanbevolen, Resolve with
  327688 and only the native package installed (the stand-in also reports an i386 build), and with the stand-ins
  storing Ubuntu's "en_US": Weergavetaal shows English (United States), no sign-out banner, Andere gebruikers › Taal
  "Engels (Verenigde Staten)", Instellingen kopiëren sends LANG=en_US.UTF-8.

### Notes for I (review)

17. native/kit/src/controls/dialog.rs (kit, frozen for E) has no way to change a button's label after `show`;
    signin.rs's `primary_button` finds the primary button by the kit's own widget name (`w11-dialog-primary`) to
    relabel it "Opnieuw proberen". Patch: in `impl Dialog`, `pub fn set_primary_label(&self, label: &str) { for (r,
    b) in self.inner.buttons.borrow().iter() { if *r == DialogResult::Primary { b.set_label(label); } } }`; then
    signin.rs calls `d_for_events.set_primary_label(..)` and `primary_button` goes.
18. native/apps/settings/src/main.rs (frozen) rebuilds only overview pages when caps.get answers after the first page
    (`shows_caps_links`). Rows of other pages that depend on a caps key that hides no page (usbguard, geoclue,
    fingerprint) must ask themselves: batch E's pages use accounts.rs `caps_now`. Optional for I: move `caps_now` to
    common.rs (`pub async fn caps_now(p: &Page) -> Value`) for every batch, or let main.rs rebuild the shown page
    whenever the first caps answer differs from Null.

### CHANGELOG lines for I (batch E, English; the release's section)

- **Accounts in Settings.** Settings › Accounts has Windows 11's pages with GNOME's functions behind them: Your info
  (picture from a file or GNOME's gallery, name), Email & accounts (GNOME Online Accounts: add, choose what each
  account is used for, sign in again, remove), Sign-in options (fingerprint with a reader, change your password with
  GNOME's strength meter, when to sign in again after being away, lock when the PC goes to sleep, block new USB
  devices with usbguard, automatic sign-in) and Other users (add, change the account type, reset the password,
  automatic sign-in, language, remove; a user who is still signed in can't be removed, as in GNOME).
- **Time & language.** Date & time sets the time and time zone automatically or by hand, syncs now and chooses what
  the taskbar clock shows; Language & region has the display language, WinPoint's own language (it can follow the
  display language), adding a language with its language packs, and the regional format with GNOME's preview, the
  first day of the week and the 12 or 24-hour clock; Administrative language settings set the sign-in screen's
  language and formats or copy yours.
- **About.** Settings › System › About shows Windows 11's device and OS specifications with GNOME's details (model,
  graphics, storage, firmware, GNOME version, windowing system, virtualisation, kernel), each with Copy, and Rename
  this PC.
