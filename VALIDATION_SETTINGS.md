# Settings validation for 0.9.1

Status on 17 November 2026: source integration, release builds and the full isolated regression run are complete.
68 isolated runs, 4695 individual checks. No unexplained scenario failure remains. A virtual monitor's lack of physical colour calibration is recorded
as an environment limit, not a successful calibration. Nothing was installed into the running desktop and nothing was pushed.

The implementation is GTK 4 and Rust. The page registry exports 85 non-data pages to Start, including capability requirements; data pages are reached through their parent. GNOME's supported functions use GNOME's settings and services, with the Windows 11 page structure and Dutch/English wording. Availability depends on hardware and installed services.

## Scope and isolation

- Heavy jobs use `tools/heavy.sh`: one machine-wide lock, at most two compiler jobs, low scheduling priority, an available-memory check and a watchdog that stops only the test/build process group.
- Build targets and caches are on the external disks. No live installation, live-session settings writes, shell restarts, administrator password or push is part of this validation.
- Host tests use `tests/rust-sandbox/run-tests.sh`, a clean environment, private HOME/XDG directories and bus, blocked system tools and a nonexistent system-bus address.
- Service integration uses `tests/dbus-fakes/run.sh`: private process/network namespaces and system/session buses, read-only host files and fresh fake services per test. Connection traces must stay inside those buses.
- GTK widget regressions use a private Broadway display inside the D-Bus sandbox. A test that could not initialise GTK is not accepted as an executed widget test.
- Practical tests use `tests/gnome/harness.py` in its private headless GNOME. Worktrees on NTFS are copied to a temporary Unix filesystem, permissions restored and test results copied back. The copy is removed afterwards.
- The harness itself establishes mode 0700 for its tool directory. Scenarios check it without repairing it. Secret API payloads in recorded results are redacted using the host's method manifests.
- A scenario's process exit code alone is insufficient: `scenario.error`, failed checks, connection/exec traces, leftover processes and socket cleanup must also be inspected.

## Final integrated evidence

| Gate | Result |
|---|---|
| Release builds | Host and native pass; 19 staged programs, SHA256SUMS verified. No new crate or package installed. |
| Host tests | 824 passed, 0 failed, 38 deliberately separate/environment-dependent tests ignored in the ordinary run; executed on a Unix copy so the strict permission checks remain intact. |
| Native tests | 250 passed, 0 failed, 1 pre-existing format/JavaScriptCore-reference comparison ignored. |
| Private service tests | 16 C network/sharing cases and 3 E account/machine/GOA cases passed; every successful traced connection stayed on the private buses. |
| Actual GTK regressions | Kit controls/breadcrumbs, Settings card extra content and oversized media artwork passed with GTK initialised on a private Broadway display. Skipping GTK initialisation is rejected. |
| Installer | 119 passed, 3 explicitly skipped (two real-install cases disabled and one absent historical ELF baseline); Unix copy, no live install. |
| Harness unit tests | 5 passed: secret recording, typed service fixtures and read-only public dictionary mounts. |
| Light gates | Rust formatting, i18n/pairs/drift/native wording, native glossary registry tests, page registry and release checks passed. Existing wording warnings are not presented as Windows verification. |
| No WebKit / GTK 3 | Source scan and all 19 staged release binaries passed. Historical records and other applications' package names remain narrowly allowlisted. |

The old per-branch ELF recording mismatch is fixed in the integrated tree and is not waived. The Gs lifetime,
real-pointer dialog/header controls, managed shortcut resets and input indicator are covered by the integrated tests.
The ordinary host test's ignored cases are not all claimed as executed: tests that require the actual user manager,
real services or unrelated historical update fixtures remain outside this no-live-session validation.

### Isolated scenario matrix

| Scenario | nl checks | en checks | Acceptance |
|---|---:|---:|---|
| `calendar-app` | 15 | 15 | Pass |
| `frosted-glass` | 28 | 28 | Pass |
| `ge1c-extension` | 58 | 65 | Pass |
| `input-bridge` | 8 | 8 | Pass |
| `kit-ui` | 12 | 12 | Pass |
| `launch-attention` | 9 | 9 | Pass |
| `lock-unlock` | 5 | 5 | Pass |
| `multi-monitor` | 75 | 75 | Pass |
| `native-apps` | 6 | 6 | Pass |
| `native-gtk-apps` | 105 | 105 | Pass |
| `native-restart` | 56 | 56 | Pass |
| `native-shell` | 44 | 44 | Pass |
| `quick-media-size` | 14 | 14 | Pass |
| `r2b-disks-race` | 8 | 8 | Pass |
| `r2d-picker` | 32 | 32 | Pass |
| `r3-late-socket` | 0 | 0 | Expected guarded early abort; zero helper connections |
| `r3-ontop` | 31 | 31 | Pass |
| `readme-explorer` | 0 | 0 | Pass |
| `readme-shots` | 13 | 13 | Pass |
| `session-mode` | 13 | 13 | Pass |
| `settings-a11y` | 251 | 262 | Pass |
| `settings-accounts` | 88 | 97 | Pass |
| `settings-apps` | 135 | 146 | Pass |
| `settings-devices` | 115 | 123 | Pass with the recorded physical calibration limit |
| `settings-input` | 164 | 171 | Pass |
| `settings-network` | 192 | 203 | Pass |
| `settings-privacy` | 108 | 118 | Pass |
| `settings-sharing` | 89 | 95 | Pass |
| `settings-system` | 225 | 237 | Pass |
| `settings-time` | 100 | 107 | Pass |
| `settings-tour` | 183 | 183 | Pass |
| `snap-accent` | 2 | 2 | Pass |
| `taskbar-app-icons` | 43 | 43 | Pass |
| `winpoint-update` | 71 | 71 | Pass |

For every normal run: private install boundary, inner exit, scenario checks, memory guard, connection/exec traces,
socket watch, socket cleanup and leftover processes were audited. The late-socket case instead must stop its own
inner session with SIGTERM when the outside socket appears; it made zero connections to that socket and the outside
helper removed it afterwards. No blanket acceptance of a scenario's old `known` failures is used.

## Practical coverage and its limits

| Group | What the isolated scenarios exercise | What they cannot establish |
|---|---|---|
| A | Mouse/touchpad controls, shortcut conflicts and reset, language/input sources, text scaling, accessibility controls and private input bridge calls. | Physical Wacom/touchscreen mapping and calibration, a real pad OSD, Orca behaviour, and desktop-portal differences on every GTK installation. Lockdown is tested with synthetic settings backends rather than real administrator locks. |
| B | BlueZ/CUPS/Avahi/colord stand-ins, device/printer data pages, authentication arguments and interactive flags, private scanner/tool launches. | Real pairing/transfer, an IPP Everywhere queue and actual jobs, scanners, Thunderbolt authorisation or a colour sensor. A virtual monitor cannot provide real calibration hardware. |
| C | Stateful NetworkManager/ModemManager stand-ins, stored credentials/settings merges, rfkill, hotspot confirmation, private sharing/RDP/SSH/PackageKit services and TLS parsing. | A real SIM/AP adapter, secret/password agents and polkit policy, an actual RDP client/keyring, real SSH/package installation or long operator scans. See the feature boundaries below. |
| D | Private Mutter layouts, keep/revert, fake UPower and backlights, private PipeWire nodes, audio controls/meters, storage/time statistics, taskbar/window behaviour. | Real GPU gamma, HDR/VRR, multiple physical displays/backlights, sound-card profiles/HDMI, charge-threshold firmware and Hyprland paths. The headless monitor reports no night-light support. |
| E | AccountsService/GOA/hostnamed/timedated/PackageKit stand-ins, local account forms, fingerprint progress, locale parsing and language changes. | Real account/password changes, OAuth sign-in, fingerprint enrolment, administrator policy or installing language packs. |
| F | Private PermissionStore/fwupd/whoopsie/insights stand-ins, app notification settings, portal change events, private index paths, privacy controls and cleanup arguments. | Real store-app permissions and Snap connections, firmware security, parental-control policy, Ubuntu reporting/security applications and every portal version. |

Settings obey the allow tables and managed-key checks. Read-only/no-control keys, including the proxy authentication switches and RDP screen-share-mode, are audited as read-only. Tests do not claim to reproduce administrator dconf locks from the host: `/etc` remains read-only.

## Explicit feature boundaries

The existing C notes remain relevant: extra VPN plugins use import/generic data fields rather than dedicated forms; VPN import uses libnm offline parsing followed by interactive AddConnection2. Newly entered APN credentials and authentication flags are also written for the attach bearer; a blank edit preserves the separately stored secrets. The form does not expose two independent credential editors. Bluetooth hotspot sharing and hotspot power-saving are not implemented. The keyring unlock interaction is GNOME's prompt rather than a separate WinPoint banner. gnome-user-share file sharing is not offered when that service is absent. Windows-only developer/Sudo/Terminal/profile switches without a GNOME counterpart are not invented.

XML slideshow backgrounds resolve a frame when loaded; automatic frame advancement is not implemented.

These boundaries must not be presented as hardware validation or as complete parity with every optional daemon. Real hardware/daemon checks are outside this isolated validation, which leaves the running session untouched.

## Wording and unchanged-session checks

`tests/i18n/test_native.py` rejects new scalar native UI and host-catalogue text that has no row in `tests/i18n/windows-glossary/*.tsv`. Its frozen pre-parity baseline is revision `af8eebdc`; keyword-only arrays are excluded. A listed row is not a claim of Windows verification: `verified=no` still needs the real Windows check.

`settings-tour.py` visits the non-data page tree, checks reset using backend `isDefault`, distinguishes an explicitly empty shortcut list from an absent setting, ignores switch-state labels when choosing controls and tests the language endonyms/dropdown. `quick-media-size.py` uses only generated local pictures and a fake MPRIS player.

`session-mode.py --session-mode winpoint` checks GNOME-look restoration, including accent-color, and unchanged dock/DING/tiling schemas, enabled/disabled extensions and mutter experimental features in the private session. No corresponding command is run in the real session.

## Informational integration notes (DF29)

KeyCapture cancellation and the final event payloads follow the API contract. The Exchange provider allow row remains
narrow. ConfigLock still requires real Unix 0600 files; tests use a Unix copy instead of weakening that check for NTFS.
Canonical application identifiers are preserved. Do Not Disturb also silences critical notification sounds, as amendment 4
requires. NoDisplay helper application names retain their host-locale behaviour. Tracker's home-folder settings are tested
only against the private HOME. The fake systemd relay is asynchronous, so its test process cannot block page updates.

## Decisions retained for the user

- GNOME's existing Alt+Tab desktop preference wins initially; the taskbar defaults to windows on the current desktop.
- The new-install language default is retained; the user can choose system display language explicitly.
- GNOME Calendar is distinguished as Agenda (GNOME) / Calendar (GNOME).
- Real Windows wording checks, real hardware/daemon checks, a live installed smoke test, pushing and release remain separate user actions.

## Historical validation

The following 0.4.0 report is preserved as historical evidence. Its HTML/WebKit measurements do not validate the current GTK 4 interface.

# Rust-instellingen — versie 0.4.0

> **Historisch (D-G72, 14 november 2026):** gemeten met de HTML-interface in WebKitGTK van toen. Die is verwijderd: Verkenner, Instellingen, Taakbeheer en de andere apps zijn nu native GTK 4-programma's, en Win11 voor GNOME bevat geen WebKit en geen GTK 3-host meer. De Rust-diensten die dit rapport controleert, bestaan nog. De laatste commit met WebKit is `6ebf169f`; zie [native/APPS.md](native/APPS.md).

Gecontroleerd op 1 november 2026 in een eigen schermloze GNOME 50.1/Wayland-sessie met drie echte Rust/WebKit-vensters van de oorspronkelijke Instellingen-app. Alle twaalf gegroepeerde integratiegevallen slagen: elf voor instellingen en één aanvullende proef van de productieprullenbakroute. De bestaande HTML/CSS-interface is behouden; de schermafbeelding staat in `preview-settings.png`.

## Native bereik en vergelijking

`settings.get`, `settings.set`, `settings.patch`, `settings.reset`, `settings.export`, `settings.import` en `settings.backupFile` worden door Rust afgehandeld. Hun antwoorden dragen `X-Win11-Service: rust-settings`. Het derde venster gebruikte een testkopie van de Python-dienst waarin alle zeven openbare instellingenmethoden bewust fouten geven. Alle zeven bleven via de Rust-host werken.

De referentie was een bevroren kopie van de oorspronkelijke Python-`SettingsStore` en methoderegistratie, met een afzonderlijk instellingenbestand. De vergelijking controleerde volledige resultaten en foutobjecten, niet alleen enkele velden:

- Standaardwaarden samenvoegen, onbekende sleutels behouden, Unicode, arraypaden en het onderscheid tussen puntpaden en letterlijke punten in een arraysegment.
- Ongeldige paden, alleen-lezen `version`, booleans tegenover getallen, ongeldige subbomen, vereiste lijsten van strings en de oorspronkelijke betekenis van lege/false waarden bij `patch`.
- Een gedeeltelijk vervangen `theme`-object blijft daadwerkelijk gedeeltelijk, ook in een ander Rust-proces en diens Python-geheugen. Export behoudt de oorspronkelijke JSON-opmaak en sleutelvolgorde.
- Import als object en tekst, foutieve import, typefiltering, behoud van onbekende instellingen, geforceerde schemaversie en reset van bekende/onbekende paden en alle instellingen.
- Back-ups bevatten dezelfde bytes als de referentie, gebruiken modus `0600`, behouden de antwoordvelden en krijgen afzonderlijke namen bij opeenvolgende verzoeken.

## Gebeurtenissen, bestaande diensten en bediening

Beide Rust-eventstreams en beide bijbehorende Python-eventstreams ontvingen de nieuwe instellingen en de juiste `changed`-paden. Het schrijvende venster behoudt het initiating-pad; een herlaadgebeurtenis kan `path: null` gebruiken. De private Python-backends namen de exacte native toestand over. Hun bestaande `files.list` gebruikte daarna aantoonbaar de nieuwe `explorer.showHidden`-waarde uit het geheugen.

De oorspronkelijke schakelaar voor transparantie werd in een echt WebKit-venster aangeklikt via zijn bestaande DOM-handler. Dit schreef de native instelling en werkte de HTML-themastatus in beide vensters bij. De afbeelding is een echte GNOME-opname van die vensters, geen nagemaakte interface.

Twaalf gelijktijdige workers schreven 24 verschillende sleutels via twee onafhankelijke Rust-hosts. Een tweede proef schreef nog 24 sleutels met Rust-verzoeken en rechtstreekse verzoeken aan beide eigen Python-backends door elkaar. Alle sleutels bleven aanwezig op schijf en bereikten beide native snapshots en beide Python-geheugentoestanden. De gedeelde bestandsvergrendeling voorkomt hierbij verloren wijzigingen.

De openbare opslag, validatie, import, export en back-up draaien native. De resterende systeemthema- en GTK-listeners blijven de oorspronkelijke compatibiliteitscode gebruiken en worden met de nieuwe toestand bijgewerkt. Deze meting beweert geen volledige Rust-migratie van alle systeeminstellingenpagina's, hardwarediensten of thema-effecten.

## Aanvullende GIO-prullenbakproef

Zonder `WIN11SHELL_TRASH_DIR` werd precies één eigen regulier bestand naar de echte GIO-prullenbakroute gestuurd. Vooraf is gecontroleerd dat de bron en de private `XDG_DATA_HOME` hetzelfde apparaatnummer hebben. Het native antwoord wees naar de eigen `data/Trash/files`, de bijbehorende `.trashinfo` en herstelnaam werden teruggevonden, en native herstellen bracht de oorspronkelijke inhoud terug. De eigen prullenbakdata en metadata waren daarna verdwenen.

Deze proef gebruikte geen productieprullenbaklijst of permanente verwijdering en geen acties op gemounte prullenbakken. De afzonderlijke suite in `VALIDATION_FILES_COMPLETE.md` controleert legen uitsluitend met een expliciete testprullenbak.

## Isolatie en grenzen

Alle appprocessen gebruikten een eigen `HOME`, XDG-configuratie, data, cache, runtime en de GSettings-keyfilebackend. De actieve desktopinstellingen en gebruikersbestanden zijn niet aangepast. De Python-referentie had bovendien uitgeschakelde systeemmutaties. De tests gebruiken geauthenticeerde HTTP, SSE en bestaande DOM-handlers; fysieke muis- en toetsenbordhardware, iedere instellingenpagina, alle bestandssystemen en langdurige foutscenario's zijn niet volledig getest. De opname bewijst geen pixelvergelijking van alle schermen.

De ontwikkelharness staat in `work/settings-native-test/run.py`, met de bevroren referentie onder `oracle/` en het volledige resultaat in `results.json`. Het compacte distributiebewijs is `validation/settings-results.json`, zonder tokens, persoonlijke instellingen, proces-ID's of bestandsinhoud.
