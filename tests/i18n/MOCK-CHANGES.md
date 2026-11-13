# Mock changes for the UI text dump

Changes to `vendor/ui/js/mock/` made so that every surface of `tests/i18n/surfaces.json` renders in mock mode without
"Onbekende functie". Mock files are not shipped behaviour: the bridge loads `mock/index.js` only with `?mock=1` or
without a host token, and the Rust host always passes a token. The first table is GE0a's (2026-11-05), on top of
`3b898e6`; the later steps' rows follow it (section "Windows Update and round 3").

| File | Change | Why |
|---|---|---|
| `vendor/ui/js/mock/gnome.js` | **New.** Mocks the 18 host methods that G's UI calls and the older mocks did not know: `software.{inspect,uninstallInfo,prepare,execute,status,cancel,jobs}`, `apps.{mimeTypes,forType,setDefaultForType}`, `explorer.{tabOffer,tabClaim,tabCommit,tabRelease,tabStatus,tabDetach,tabPosition}`, `nativeBackdrop.configure`. | The App-installatieprogramma, Standaard-apps, the "Verwijderen" items in Start and in Instellingen › Apps › Geïnstalleerde apps, and Verkenner's tab transfer showed "Onbekende functie" in mock mode. |
| `vendor/ui/js/mock/index.js` | `FILES` gets `'gnome'` (appended last). | Loads the new file. Appending keeps the order of the other mock modules, their `start()` calls and so their seeded random streams. |
| `vendor/ui/js/mock/index.js` | An unknown method now also logs `console.warn("[mock] unknown method <name>")` before it throws the same `unknown_method` error with the same Dutch message. | `tests/i18n/run.sh --notes FILE` lists these warnings per surface, so a surface that calls a method no mock serves is found even when the UI swallows the error. `?debug=1` shows only `console.error`, so no visible text changes. |

## Windows Update and round 3 (GW3, GW4, GX1, R3-UI)

Rows asked for by GW3 decision D4 and GX1 n9 (plan.md D-G41, D-G45), written in round 3 (R3-UI), and R3-UI's own.

| File | Change | Why |
|---|---|---|
| `vendor/ui/js/mock/update.js` | **New (GW3).** Every `update.*` method of `notes/claude/windows-update-gnome.md` §4.3 with the Rust service's snapshot per state: `?updateScenario=` picks one of §9.3's ids (H01-H28, R01-R06, plus GW3's lettered text variants); without it the page starts in H03. Live flows on a click (check, install with simulate, confirm, download with `update.progress`, install, restart; cancel, repair, `update.release.attach`), the events of §4.4, and the host texts of `host/i18n/update.tsv` in the language of the call, with `key` and `params` (C2; `tests/i18n/test_update_mock_rows.py` compares the rows). **GW4** added `update.history`, `update.options` and `update.setAutoOpen` with their own states (S01-S03i: history groups and paging like `history.rs`, the options rows like `options.rs`). **R3-UI** added S03j (snapd holds the refreshes for 72 hours: `snaps.held`, `next` = the hold's end) and S03k (held with no end, `next` null), inside the GW4 section. | The Update page, Updategeschiedenis and Geavanceerde opties need the service of GW2 (`host/src/services/update*`), which the mock replaces: surfaces `settings-update.*`, `settings-update-history.*`, `settings-update-advanced.*` and the two registry pages. |
| `vendor/ui/js/mock/system.js` | `system.updates` gains `rebootRequired: false` (GW3, additive, §4.10). | The real `system.updates` reports it; nothing in the dump shows it. |
| `vendor/ui/js/mock/settings.js` | **GW3:** `DEFAULTS.update` = `{notifyRestart: true, metered: false, autoOpenPrevious: null, ubuntuHoldPrevious: null}` (§4.12, as `host/src/settings-defaults.json`). **R3-UI (D-G31):** `settings.set`/`settings.patch` refuse a language string other than `"nl"`/`"en"` (and a path below `language`) with code `invalid_value`, the host's text "Deze waarde past niet bij deze instelling." and key `settings.badValue`, as `host/src/settings.rs` does since D-G24; a value of the wrong type stays `bad_value` (checked first in a patch, as the host does). `settings.import` drops an invalid language and keeps the current one; the rest of the file is taken as before. | Geavanceerde opties binds its two switches to `settings.update.*`; D-G31 aligns the mock's refusal with the host's. |
| `vendor/ui/js/mock/shell.js` | **GX1:** `shell.restartAll {reason}` with the host's guard (release upgrader, an own update job in any phase, an active Software job; texts as `host/i18n/update.tsv` in the settings language), the event `shell.restartAll {nonce, origin, at, reason}`, the mock host away for the origin's 2 s and back with the new libraries (the Update page's Win11 restart InfoBar is gone), and a second call during that restart answering the same nonce with `pending: true` and no event (GX1 review M1). **R3-UI:** the simulated host outage's `offline` error (`?mockHostDown=offline`, `__win11MockHost`) is `core/bridge.js`'s text in the UI language instead of always Dutch. **R3-UI fix round:** `?mockRestartAll=live` makes `shell.restartAll` answer `dryRun: false` (also its `pending` answer), as a host outside test mode does; without it every answer stays a dry run. | GW3's D3 (the InfoBar's "Win11 Shell opnieuw starten" button) and GX1's restart; the en dump's console notes showed the Dutch offline text (plan.md round 2 list); the switch lets surface `settings-check.restartall-sent` show the button that stays disabled after a real answer (gx1.md §7 decision 1), which a dry run never reaches. |
| `vendor/ui/js/mock/index.js` | **GW3:** `update` appended to `FILES`. **R3-UI (D-G41):** a thrown error keeps `key` and `params` (copied like `core/bridge.js` does), and since the R3-UI fix round C2's `data` too (`settings.set`'s `{path}`, `settings.patch`'s `{paths}`; `tests/i18n/mock_index.test.mjs`), so the Update page's refusals carry their catalogue key in mock mode too; the mock's own two texts ("Onbekende functie: …", "Er ging iets mis.") follow the UI language ("Unknown function: …", "Something went wrong."). | Contracts C2: the UI matches errors on code and key; mock refusals had only code and message. |

### WU-FIX (D-G46, D-G49, D-G51)

| File | Change | Why |
|---|---|---|
| `vendor/ui/js/mock/update.js` | `update.release.check` answers as `release.rs` `check()` does: `{started:true}` (then `release.lastCheck` and an `update.changed` 0.8 s later), else `{started:false, skipped}` for Prompt=never, offline, metered without `settings.update.metered`, a run in the last 24 h (`recent`) or one still running (`running`); refused while Ubuntu's upgrader runs. `update.install` during a check refuses with its own row `update.err.checking` (code `busy`, as `pk.rs`); the host-rows block gains it and `update.options.err.autoOpen`. | GW5 M1: the page's "Controleren op updates" also calls `update.release.check`; GW2b n6 and GW4 m8: the two new host rows (`tests/i18n/test_update_mock_rows.py`). No surface shows them, so no dump block changes. |
| `vendor/ui/js/mock/settings.js` | `settings.import` of a file without a `language` keeps the current language too (D-G49), as `host/src/settings.rs` `import()`; `tests/i18n/mock_settings.test.mjs` checks both. | The host and the mock agree on every import (R3-NATIVE review n1). |

## How `mock/gnome.js` behaves

* **Sources.** Shapes and host texts come from the Rust host: `host/src/services/software.rs` with
  `software/inspect.rs` and `software/resolve.rs`; `host/src/apps/pertype.rs`; `host/src/transfers.rs`;
  `host/src/runtime.rs`. Host texts follow `settings.language`, like `mock/files.js` and the Rust host's
  `t(lang, nl, en)`. With `?lang=en` and Dutch mock settings they therefore stay Dutch, like every other mock host
  text in `baseline-en.txt`.
* **Installation files.** `software.inspect` knows the extensions `.deb`, `.AppImage`/`.appimage`, `.flatpakref`,
  `.flatpak` and `.snap`, with fixed example metadata (Visual Studio Code, Kdenlive, hello-world). Any other file
  gets the host's "Dit bestand is geen ondersteund installatiepakket.", and a path that is not absolute gets "Dit
  bestand kan niet worden gelezen.".
* **Removal.** `software.uninstallInfo` follows the host's rules:
  * own `win11shell-*` apps are blocked as protected;
  * snap ids (`x_y.desktop`) go to snapd, and `snap-store` and `firmware-updater` are blocked as part of the base
    system;
  * GIMP, OBS Studio and Kdenlive go to Flatpak;
  * Steam, NVIDIA Settings and Blender are blocked with "installatiebron … niet betrouwbaar";
  * Nautilus is blocked as essential;
  * everything else is a PackageKit package ("Verwijdert het pakket <pkg>");
  * an id that is not in `apps.list` is "niet meer geïnstalleerd".
* **Plans and jobs.** `software.prepare` makes single-use plans. `software.execute` starts a **dry run** in 400 ms
  steps: queued, then running 35 %, then 80 %, then completed with "Voorbeeld voltooid. Er is niets geïnstalleerd of
  verwijderd.". It emits `software.changed` at every step. `software.cancel` works while the job runs. Nothing is
  installed or removed. `?mockJobs=1` starts the activity list with two finished jobs: a completed preview install
  of Visual Studio Code and a failed Kdenlive removal (surface `installer.activity`).
* **Default apps per file type.** `apps.mimeTypes` serves 11 file types (PDF, ODT, tar.gz, exe without any app, zip,
  MP3, JPEG, PNG, HTML, plain text, MP4) with Dutch or English names and the app briefs of `mock/apps.js`.
  `apps.forType` lists the default first. `apps.setDefaultForType` changes the default in memory and answers
  `effective: true`.
* **Tab transfer.** Tickets are kept in memory. The error texts are the host's Dutch strings, with code
  `tab_transfer`. `tabDetach` opens no second window.
* **`nativeBackdrop.configure`** answers `{available: false}`. The UI asks for it only under the Rust host
  (`window.__win11shellHost.nativeBackdrop`), never in mock mode.

## Effect on the dump

* A dump with and without these changes was compared with `blocks.py` (2026-11-05). Only the 18 G-only surfaces
  that call the new methods differed: `shell.start.menu`, `shell.allapps.menu`, `settings.installed.menu`,
  `settings.installed.review`, the four `defaultapps*` surfaces and the ten `installer*` surfaces. Without the mock
  they showed "Onbekende functie: …". All other 121 blocks were byte-identical.
* **The UI manifest no longer matches.** `python3 scripts/verify-ui.py` now fails: `js/mock/gnome.js` is extra and
  `js/mock/index.js` changed. As a result `scripts/package.py` refuses to package until `vendor/ui-sha256.json` is
  regenerated and `vendor/ui-patches.json` lists these files. GE1a already has to do this for the UI port
  (gnome-map §2.5 step 4). No package is built before then.
