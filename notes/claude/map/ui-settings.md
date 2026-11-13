# Map: Settings UI, Standaard-apps app and installer app (G) — i18n state and porting from H

Scope: `vendor/ui/js/apps/settings/**` (32 files), `vendor/ui/js/apps/defaultapps/main.js`,
`vendor/ui/js/apps/installer/main.js` and their CSS (`css/settings.css`, `defaultapps.css`, `installer.css`, `software.css`)
in G (`/home/robin/Documents/Codex/2026-11-01/chat/outputs/win11-gnome`, branch `claude/gnome-english-update`), compared with
H (`.../scratch-2026-10-25-a089b6`). This was a read-only pass. Nothing in G or H was changed. I did not run a build or open
any UI.

Method: I ran H's own static extractors (`tools/i18n/jsdicts.mjs`, `tools/i18n/scan_ui.mjs` with H's
`tests/i18n/dutch-words.txt`) through scripts in my scratchpad against G's files. I added a scan for English-looking
literals outside the dictionaries, and SHA-256 comparisons against `vendor/ui-original-sha256.json` and H's pre-step
snapshots (`dist/i18n-snapshots/E0a`, `E1`). I also tried `git merge-file` on copies in the scratchpad.

## Summary

* **The Settings UI in G is already fully bilingual. No new mechanism is needed.** It has 31 `makeT` dictionaries in
  scope with **790 nl keys and 790 en keys (100 % parity, 0 one-sided keys)**. The registry has 34 pages, each with a
  nl/en title, description and keyword list. Language switching is live (`main.js:291`). The only real hard-coded Dutch
  text is one fallback filename (`update.js:159`), and the host's own Dutch name overrides that fallback anyway.
* **The language setting exists**: Instellingen › Tijd en taal › Taal en regio (`pages/time.js:120-143`). It offers
  `nl` "Nederlands" and `en` "English (United Kingdom)", writes `settings.language`, and the default is `nl`
  (`host/src/settings-defaults.json:3`, `core/theme.js:319-320`). Nothing has to be built for it.
* **G's vendored settings files are byte-identical to H's state before step E1** (the SHA-256 of every original file
  matches `ui-original-sha256.json` and H's `E1` snapshot). So H's E1 work ports cleanly:
  * 21 files are identical in G and H, so there is nothing to port;
  * 7 files were changed only in H and can be copied as they are (`about`, `bluetooth`, `display`, `startup`, `taskbar`,
    `time`, `update`);
  * 2 files were changed in both (`installed.js`, `registry.js`), and both trial 3-way merges came out with **0 conflicts**;
  * 2 files were changed only by Codex (`power.js`, `background.js`), so G keeps them.
* **Codex's pages follow the makeT pattern**: `power.js` (73/73 keys), `installed.js` + `core/software.js` (39/39), the
  power entry in `registry.js`, and the new apps `defaultapps/main.js` (26/26) and `installer/main.js` (21/21). Both new
  apps re-label themselves when the language changes. The only defect found is a duplicate English heading in power.js:
  "Power" twice (`power.js:57` and `:78`, nl "Energie" / "Aan/uit").
* **What English mainly still lacks is host text.** `showError()` (`common.js:71-74`) and the other host-text paths
  show `error.message` from Rust as it is. The Rust host has a per-call `ctx.language()` (`host/src/services/mod.rs:62-69`)
  that follows `settings.language`:
  * already bilingual: Bluetooth, Wi-Fi failures, storage names, the app catalog and the software service;
  * **Dutch-only**: the errors from energy/power, display, brightness, audio, network, settings, apps (defaults, per-type,
    launch, recent), tray, clipboard and notifications (~100 `ApiError::new` sites, grep estimate);
  * the settings-export file name (`host/src/settings.rs:136`).
* **GNOME-specific wording is wrong in both languages.** On G, `clipboard.list` always returns `supported:false`
  (`host/src/services/shell/clipboard.rs:30`). The Clipboard page therefore always tells the user to sign out and pick
  the "Win11 Shell" session (`clipboard.js:23/41`), which does not exist in the GNOME version. The Display and Night
  light notes say the same kind of thing (`display.js:26/38/62/74`), and the host adds "Daarvoor zijn Hyprland en
  hyprsunset nodig" (`host/src/services/system/power.rs:154`). Fixing this changes Dutch text, so Robin has to decide.
* **Every UI edit must be recorded in `vendor/ui-sha256.json` and `vendor/ui-patches.json`**: `scripts/verify-ui.py`
  fails on any hash that differs from the manifest, and `scripts/install.py:72` ships those manifests.
* **Estimate for this area:**
  * UI port and fixes: about 5 agent-hours;
  * host message work for the Settings pages: about 7–9 h (overlaps with the host mapping);
  * baseline and English dump tests plus visual checks: about 7–9 h.

## Findings

### (a) How strings are written, per file

Legend: `mT` = number of makeT dictionaries; `nl`/`en` = keys; `pairs` = inline `{nl,en}` objects or registry `P()` calls;
`hard` = string literals with a Dutch word outside any language data (H's ratchet scan). I also scanned for English-looking
literals outside the language data. Apart from SVG markup, CSS class names and a developer error (`main.js:176`), I found
none in this area.

| File (G `vendor/ui/js/apps/…`) | lines | mT | nl | en | pairs | hard | Notes |
|---|---:|---:|---:|---:|---:|---:|---|
| settings/main.js | 325 | 1 | 10 | 10 | 0 | 0 | Titles via `tr(page.title)`; live switch `:291-299` |
| settings/common.js | 420 | 1 | 13 | 13 | 0 | 0 | `ct` shared dictionary; `showError` shows host `error.message` (`:71-74`) |
| settings/registry.js | 243 | 0 | 0 | 0 | 102 | 1 | 34 `P()` pages (title/desc/keywords nl+en); `'datum-tijd'` is an alias id (`:185`), not text |
| settings/colorpicker.js | 137 | 1 | 11 | 11 | 0 | 0 | |
| settings/navicons.js | 102 | 0 | 0 | 0 | 0 | 0 | SVG only |
| pages/about.js | 107 | 1 | 26 | 26 | 0 | 0 | `cores: '{c} kernen, {t} threads'` is not a plural (`:78`); H fixed it |
| pages/accessibility.js | 112 | 1 | 21 | 21 | 0 | 0 | |
| pages/accounts.js | 66 | 1 | 15 | 15 | 0 | 0 | |
| pages/apps.js | 11 | 0 | 0 | 0 | 0 | 0 | Link cards only (registry titles) |
| pages/background.js | 182 | 1 | 24 | 24 | 0 | 0 | Codex: one logic line (`:121`); `:94` strips "(licht\|donker\|light\|dark)" from names |
| pages/bluetooth.js | 1035 | 1 | 92 | 92 | 0 | 0 | Matches the host's Dutch/English text: `/reageert niet\|not responding/` (`:233`), `/geannuleerd\|cancel/` (`:509`) |
| pages/clipboard.js | 84 | 1 | 16 | 16 | 0 | 0 | `na` wording is wrong for GNOME (see Risks) |
| pages/colors.js | 160 | 1 | 25 | 25 | 0 | 0 | |
| pages/defaultapps.js | 99 | 1 | 23 | 23 | 0 | 1 | `'WebBrowser'` is a freedesktop category (`:46`), not text; sentence built with `t(k.kind).toLowerCase()` (`:77`) |
| pages/desktopicons.js | 55 | 1 | 18 | 18 | 0 | 0 | |
| pages/display.js | 372 | 1 | 34 | 34 | 0 | 0 | `keepBody` has no plural (`:40/:76`); title composed with `·` (`:248`); H fixed both |
| pages/home.js | 213 | 1 | 26 | 26 | 0 | 0 | |
| pages/installed.js | 139 | 1 | 24 | 24 | 0 | 1 | `CAT_NL`/`CAT_EN` tables (`:53-59`); `'WebBrowser'` category id (`:55`); Codex removed `uninstall`/`uninstallDesc` |
| pages/network.js | 309 | 1 | 48 | 48 | 0 | 0 | |
| pages/notifications.js | 80 | 1 | 13 | 13 | 0 | 0 | |
| pages/personalization.js | 30 | 1 | 1 | 1 | 0 | 0 | |
| pages/power.js | 327 | 1 | 73 | 73 | 0 | 0 | Codex rewrite (H: 130 lines, 37 keys); en "Power" appears twice (`:57`, `:78`) |
| pages/privacy.js | 83 | 1 | 20 | 20 | 0 | 0 | |
| pages/sound.js | 130 | 1 | 14 | 14 | 0 | 0 | |
| pages/start.js | 121 | 1 | 20 | 20 | 0 | 0 | |
| pages/startup.js | 67 | 1 | 11 | 11 | 0 | 0 | |
| pages/storage.js | 222 | 1 | 35 | 35 | 0 | 0 | `catName` falls back to the host's `c.name` (`:95`) |
| pages/system.js | 105 | 1 | 12 | 12 | 0 | 0 | |
| pages/taskbar.js | 391 | 1 | 29 | 29 | 0 | 0 | `Intl.Collator('nl', …)` (`:72`) |
| pages/themes.js | 153 | 1 | 13 | 13 | 14 | 0 | Theme and wallpaper names as `{nl,en}` pairs via `tr()` |
| pages/time.js | 145 | 1 | 33 | 33 | 0 | 0 | Language page; picker labels are endonyms (`:122`) |
| pages/update.js | 221 | 1 | 43 | 43 | 0 | 1 | **`win11shell-instellingen-${stamp}.json` fallback (`:159`)** is the only real hard-coded Dutch in scope |
| defaultapps/main.js (Codex) | 157 | 1 | 26 | 26 | 0 | 0 | `onLangChange` re-labels and reloads (`:154`) |
| installer/main.js (Codex) | 144 | 1 | 21 | 21 | 0 | 0 | `onLangChange` (`:134-138`); job and plan text comes from `core/software.js` (39/39) and the host |
| **Total** | | **31** | **790** | **790** | **116** | **4 (1 real)** | |

* **Identical en and nl values** (31) are all valid loanwords: "Bluetooth", "Status", "Downloads", "Volume", "Kernel",
  "Model", "Widgets" and similar. No en value contains a word from H's Dutch word list.
* **Language ternaries:** none in scope. `installed.js:59` chooses `CAT_EN`/`CAT_NL` by `lang()`, a table pair as the
  convention allows.
* **Collation through `localeCompare`/`Intl.Collator('nl')`** happens at 5 sites: `installed.js:108`, `bluetooth.js:997`,
  `startup.js:41`, `registry.js:234` and `taskbar.js:72`. They give the same result in nl and en, and H switched all five
  to `collator()`.
* **CSS**: `settings.css` (872 lines), `defaultapps.css` (34), `installer.css` (19) and `software.css` (11) contain no
  literal `content:` text and no `attr()`. 14 rules use `white-space: nowrap` with an ellipsis (tile titles, device
  names, hero names). Longer English text gets truncated there instead of wrapping (for example "Bluetooth & devices"
  tiles). I did not check this visually.

**registry.js titles, keywords and search, per language** (loaded through a copy in the scratchpad):
* **Content:** 34 pages (11 top-level), and every title and description has nl and en. There are 285 nl keywords and
  271 en keywords, plus 78 deep-link aliases (bilingual input ids, kept by design).
* **Codex's change:** the `power` entry (`registry.js:38-45`) got a new nl/en description and more keywords in both
  languages.
* **How search works:** `searchSettings(q, lang)` (`registry.js:~200-236`) is called with the UI language from
  `main.js:104` (suggest box) and `main.js:182` (results page). It scores title, keywords and description in that
  language, and falls back with less weight to the other language's title (25) or keyword (15) (`:207`).
* **Probes:**
  * "helder" in nl finds display;
  * "brightness" in en finds display (60); in nl it still finds display (15), through the fallback;
  * "power mode" in en finds power (110); "taal" and "language" find language (80).
* **Gaps, in both languages:**
  * "standaardapps" (nl, without hyphen) finds nothing, because the title is "Standaard-apps";
  * "file type" and "bestandstype" find nothing;
  * Codex's new Standaard-apps (per file type) app and the App-installatieprogramma have no registry entry or keyword,
    so Settings search cannot find them.

### (b) Pages that show host-made text

All pages show host errors through `showError(r.error)` → `error.message` (`common.js:71-74`), through `run()`
(`common.js:82-87`), or directly (`e.message` in `defaultapps/main.js:75,109,143` and `installer/main.js:78,85,91,118`).

How the Rust host picks a language:
* `ServiceContext::language()` (`host/src/services/mod.rs:62-69`) reads `settings.language` fresh from disk on each call
  (`SettingsStore::snapshot`, `host/src/settings.rs:77-80`).
* There is **no per-request language** (no `X-Shell-Lang`), so a `?lang=en` test window against a Dutch host shows
  mixed text.

| Page / app | Host methods | Host text shown to the user | Rust language state |
|---|---|---|---|
| bluetooth.js, home.js | `system.bluetooth.*` | Error and result messages (`res.error`, `note.message` `:629`), device names (data) | **Bilingual**: `system/strings.json` (33 nl / 33 en) via `text()` (`services/system.rs:485-494`); codes such as `not_ready` and `cancelled` exist (`system/bluetooth.rs:1255-1270`) |
| network.js, home.js | `system.network.*` | `res.value.error` (`network.js:123`), `p.name`, SSIDs | Wi-Fi failures **bilingual** (`wifi_fail`, `system/network.rs:736-757`, 12 nl/en pairs); 7 `ApiError` sites **Dutch-only** (for example "Ongeldig wachtwoord.", `:336`); "Verborgen netwerk" is bilingual; NetworkManager connection names are data in the system locale |
| power.js | `system.power.settings.*`, `system.battery.get`, `system.power.action` | `showError(result.error)` (`:246`), `er.error?.message` (`:305`), raw profile ids for unknown profiles (`:208,:210`), raw action name (`:157`) | **Dutch-only**: `system/energy.rs` (13 sites, for example `:100,:131,:356`) and `system/power.rs` (8 sites, for example `:53,:90,:116`) |
| display.js | `display.*`, `system.brightness.*`, `system.nightLight.*` | Errors; monitor `description`/`name` (EDID data) | **Dutch-only**: `services/display.rs` (8 sites, Hyprland-only), `system/hardware.rs` (4), night light `system/power.rs:154,195` ("…Hyprland en hyprsunset nodig") |
| sound.js | `system.audio.*` | Errors; sink names (PipeWire, system locale) | **Dutch-only**: `system/audio.rs` (6 sites, `:79-157`) |
| storage.js, home.js | `storage.volumes`, `storage.categories` | Volume names ("Systeemschijf", "{size}-schijf"), category names when there is no UI key | **Bilingual** `_STR` in `storage/data.json`, used through `ctx.language()` (`storage/categories.rs:206-209`) |
| installed.js | `apps.list`, `apps.launch`, `software.uninstallInfo` | App names and keywords; launch errors; removal reasons (`core/software.js:106,108`) | Names **localised by settings language** (`apps/catalog.rs:471,501`; Codex validated nl/en, `VALIDATION_APPS_CATALOG.md:7,24`); launch errors Dutch-only (`apps/mod.rs`, 7 sites); software plans **bilingual** `err(lang, code, nl, en)` (47 pairs in `services/software*.rs`) |
| defaultapps.js (page) | `apps.defaults`, `apps.list`, `apps.setDefault` | App names; errors; `overriddenBy` file paths (data) | Names localised; errors **Dutch-only** (`apps/defaults.rs`, 10 sites, for example "Onbekend soort standaard-app." `:302`) |
| defaultapps/main.js (app) | `apps.mimeTypes`, `apps.forType`, `apps.setDefaultForType` | MIME type labels (`item.name`, `detail.name`), app names, `failure.message`/`e.message` | Labels use `xml:lang` = the settings language (`apps/pertype.rs:70-103`), then the untagged comment, `{s}-koppelingen` / `{s} links`, and last `gio::content_type_get_description` (**process locale**, so English on Robin's en_US machine); errors **Dutch-only** (`apps/pertype.rs`, 7 sites) |
| installer/main.js (app) | `software.inspect`, `software.uninstallInfo`, `software.jobs` | `info.name`, `info.description` (package metadata), `info.notes`, `info.reason`, `job.message`, `job.error.message`, `e.message` | Notes, reasons and errors **bilingual** pairs (`software/inspect.rs:78,85`, `software/common.rs:20`); PackageKit messages are raw English (`software/packagekit.rs:191,221-231`); I/O errors are raw `e.to_string()` (`software.rs:83`, `software/resolve.rs:132,150`) |
| startup.js | `sysmon.startup`, `sysmon.setStartup` | Autostart names | Localised by settings language (`sysmon/startup.rs:238` test) |
| update.js | `system.updates`, `system.openUpdater`, `settings.export`/`import`/`reset`/`backupFile` | **`r.value.name` export file name** (`update.js:159`), errors, backup path | Export name **always Dutch** (`host/src/settings.rs:136`; a test checks the prefix at `:1034`); settings errors Dutch-only (`settings.rs`, 8 sites); "Er is geen programma voor updates gevonden." (`system/power.rs:116`) |
| background/colors/themes/taskbar/time/privacy/accessibility | `settings.patch`, `shell.pickFile`, `tray.list`, `system.time`, `apps.clearRecent` | Errors; tray item names (data); time zone id (data) | Errors Dutch-only: `settings.rs`, `services/tray.rs` (19 sites), `apps/recent.rs` (1), `services/system.rs:497` ("Ongeldige waarde.") |
| clipboard.js | `clipboard.*` | Errors | `shell/clipboard.rs` always unsupported on G (`:30`); 2 Dutch-only sites |
| notifications.js | `notifications.list`/`test` | The test notification | **Bilingual** (`services/notifications.rs:514`). Nameless notifications get `appName "Melding"` (`:361`), but that shows in the notification centre, not on this page |
| about/system/accounts | `system.info` | OS, kernel, CPU, user names (data); desktop name | `desktop_gnome` comes from strings.json (bilingual) |

Also relevant: `core/bridge.js` (outside scope, but behind every Settings error) has **15 Dutch-only strings**, for
example `'Verbonden'` (`:65`), `'Geen verbinding met Win11 Shell…'` (`:152`) and `'Onverwacht antwoord…'` (`:159`). H's
E1 moved them into `makeT`.

### (c) The language setting page

* **Where it lives:** `pages/time.js:120-143` (page id `language`, alias `lang`/`region`, `registry.js` "Taal en regio" /
  "Language & region").
* **The choice:** a `Dropdown` with `nl` "Nederlands" and `en` "English (United Kingdom)" (`:122`). It writes
  `settings.language` through `setSetting` and resets on error (`:124`).
* **What the page shows around it:**
  * a note "Apps van Ubuntu gebruiken hun eigen taalinstelling." (`:30/:54`);
  * a live "Notaties" preview (date, time, number, size via `core/format.js` with `locale()` = nl-NL / en-GB);
  * "Taal: {l}" on the Tijd en taal hero (`:93`).
* **Plumbing:** `core/i18n.js` supports exactly `['nl','en']` (`:10`), `?lang=` wins for testing (`:14-17`), and
  `theme.js:319-320` calls `setLang(forcedLang || s.language || 'nl')` on every settings change.
* **The host side:**
  * The host accepts any value. `ctx.language()` maps everything except `"en"` to `nl`, and the UI ignores unknown
    values.
  * The host keeps the default `"language": "nl"` (`host/src/settings-defaults.json:3`).
  * Every process polls settings.json every 150 ms and emits `settings.changed` (`host/src/settings_runtime.rs:75-88`),
    so every open surface switches.
* **Not there:**
  * a regional-format choice separate from the language (H's optional E9);
  * a language choice in `scripts/install.py` (no `--lang`);
  * any sign-out or restart hint. None is needed, because the UI switches live.

### (d) Pages Codex added or changed

From `vendor/ui-patches.json`, confirmed by SHA-256: in scope these are exactly `background.js`, `installed.js`,
`power.js` and `registry.js`, plus the added `apps/defaultapps/main.js`, `apps/installer/main.js`, `css/defaultapps.css`,
`css/installer.css`, `css/software.css` and `core/software.js`.

* **power.js:** rewritten for GNOME energy profiles, screen timeout and sleep on AC/battery (`EnergySettings`,
  `:167-268`). It follows the pattern fully: 73/73 keys, plural `seconds` (`:27/:70`), and durations through
  `format.duration`. Codex checked it in en as well (`../../work/native-energy-test/energy-en.png`, `chrome-en.log`).
  Issues:
  * en `energy: 'Power'` and `powerGroup: 'Power'` give two groups with the same English heading (nl "Energie" /
    "Aan/uit"), visible in `energy-en.png`;
  * unknown profile and action ids are shown raw (`:157,:208,:210`);
  * host errors are Dutch.
* **installed.js:** the disabled "Verwijderen" item became the native removal flow from `core/software.js`
  (`:9,:88-97`). The removed keys are no longer used. `core/software.js` follows makeT (39/39).
* **background.js:** one condition changed (`:121`), with no strings.
* **registry.js:** the power entry, nl and en together (`:38-45`).
* **defaultapps/main.js and installer/main.js:** new, makeT, and they follow language changes. Their CSS has no text.

### (e) Comparison with H (E0 + E1)

| Group | Files | Action |
|---|---|---|
| Same in G and H (G = original = H) | colorpicker, common, main, navicons, accessibility, accounts, apps, clipboard, colors, defaultapps (page), desktopicons, home, network, notifications, personalization, privacy, sound, start, storage, system, themes (21) | Nothing to port |
| Changed only in H (G = original) | about.js (`nCores`/`nThreads` plurals), bluetooth.js (`isNotResponding` and cancel by code, not text; `collator()`), display.js (`scaleLayoutN`, plural `keepBody`), startup.js (`collator()`), taskbar.js (`collator(COLL_OPTS)`), time.js (endonym "Nederlands (Nederland)"), update.js (`exportName` key) | Copy H's file. bluetooth/startup/taskbar import `collator` from core/i18n.js, so H's `core/i18n.js` has to come along (G's copy = original, so that is a clean copy too) |
| Changed in both | installed.js (Codex: software removal; H: `collator`, `:5`, `:106-109`), registry.js (Codex: power entry; H: `import { collator }` and the sort `:227-232`) | `git merge-file` with H's E1 snapshot as base: **0 conflicts** for both (trial in the scratchpad) |
| Changed only by Codex | power.js, background.js | Keep G (H's power.js is the original 130-line page) |
| Not in H | defaultapps/main.js, installer/main.js, core/software.js | Already bilingual; they only need tests |

* **Compatibility of H's changes with G's Rust host:**
  * H's bluetooth.js drops the Dutch-text regexes and relies on `e.code === 'not_ready'`, `res.cancelled` and
    `res.code === 'cancelled'`. G's Rust host emits all three (`system/bluetooth.rs:453-477,1255-1270`). H's extra
    `e.key === 'system.bt_not_responding'` never matches in G, which is harmless.
  * H's update.js `exportName` key only applies when the host returns no name. G always returns the Dutch name, so the
    host has to change as well (H did this in `i18n/host/settings.tsv:21`).
* **Core files the Settings UI depends on:**
  * `core/i18n.js`: G = original; H adds `countOf`, `list`, `collator` and `i18nReport`, so copy it.
  * `core/format.js`: G = original; H's change is 24 lines (cached `Intl`), so copy it.
  * `core/bridge.js`: G changed 1 line (settings icon) and H 104 lines; 0 conflicts.
  * `core/theme.js`: G changed 48 lines and H 4 lines (language in the theme cache); 0 conflicts.
* **Effect of `X-Shell-Lang` on G:** H's bridge.js adds the header. Calls are same-origin (`/api/call/…`,
  `core/bridge.js:143-145`), so there is no CORS preflight problem. G's host ignores the header, and the backend proxy in
  `host/src/runtime.rs:391-402` forwards only the token and Content-Type.
* **Dutch text that H's E1 changes in these files:**
  * `time.js` picker label "Nederlands" → "Nederlands (Nederland)": visible on the language page; H's D6 recorded it as
    intended;
  * a count of 1 now reads correctly in Dutch: "1 kern", "over 1 seconde";
  * everything else is byte-identical in nl.

  Under Robin's rule "Dutch stays unchanged", the endonym change needs an explicit yes, or it can be left out.

## Work needed

Estimates are agent-hours, for this area only.

1. **Port the core i18n support.** Copy H's `core/i18n.js` and `core/format.js` into `vendor/ui/js/core/` (clean copies;
   G = original). **0.5 h.**
2. **Port H's E1 Settings changes.**
   * Copy about, bluetooth, display, startup, taskbar and update.
   * Copy time.js only if Robin accepts "Nederlands (Nederland)". If not, take H's file but keep "Nederlands".
   * 3-way merge installed.js and registry.js.

   **1.5 h** including a mock-mode smoke test.
3. **Port the bridge and theme changes.** 3-way merge H's `core/bridge.js` (Dutch-only connection errors → makeT,
   `e.key`/`e.params`) and `core/theme.js` (language in the theme cache). These are outside this area, but they decide
   the language of every Settings error toast. **1 h.**
4. **Fix the duplicate heading in power.js.** Give the second English "Power" heading (`power.js:78`) another name,
   keeping the nl text. Raw profile/action ids can stay (fallback only). **0.25 h.**
5. **defaultapps.js sentences.** Replace `t(k.kind).toLowerCase()` in sentences (`defaultapps.js:77`) with per-kind
   phrases, with the nl output unchanged. **0.5 h** (optional).
6. **Registry search** (additive keywords, nl and en).
   * Add "standaardapps", "bestandstype(n)", "file type(s)", "extensie" to `defaultapps`.
   * Decide whether the per-type Standaard-apps app and the installer get a link card or entry.

   **0.5–1 h** (needs a choice from Robin or a coordinator).
7. **GNOME wording** for the Clipboard `na`, Display `displaysNA`/`nightNA` and host night light text.
   * Rewrite for GNOME in nl and en, for example point to GNOME Settings › Displays › Night Light.
   * This changes Dutch text, so it needs Robin's decision first.

   **1 h UI + 0.5 h host.**
8. **Host settings-export name** per language (`host/src/settings.rs:136`, test at `:1034`). **0.5 h.**
9. **Host error messages behind the Settings pages.** Make the ~100 Dutch-only `ApiError::new` sites bilingual: energy,
   power, display, hardware, audio, network (non-Wi-Fi), settings, apps (defaults, per-type, mod, recent), tray,
   clipboard, notifications and `system.rs` `number_arg`. Use one Rust helper: either the `err(lang, code, nl, en)`
   style Codex used in `services/software/common.rs:20`, or `strings.json` keys like `system.rs:485`. **6–8 h.**
   Coordinate with the host mapping so the helper is built once.
10. **Tests.**
    * Port H's Dutch baseline dump (`tests/i18n/dump.html`, `serve.py`, `run.sh`, `surfaces.json`: 35 Settings surfaces)
      and the ratchet `scan_ui.mjs` (it hard-codes `ROOT/ui/js`) to G's `vendor/ui` and its Codex-changed mocks.
    * Record nl **before** any port step, then check nl byte-identity and English completeness after each step.

    **4–6 h.**
11. **English visual pass.** Check all 34 Settings pages and the two apps in en for Win11 look: truncation in the 14
    `nowrap` rules, dropdown widths 200–260 px (for example power.js `:170-179`, time.js `:123`). Use the nested session
    or mock screenshots, never the live display. **2–3 h.**
12. **Manifests.** Regenerate `vendor/ui-sha256.json`, extend `vendor/ui-patches.json` (Dutch prose, like Codex's) and
    run `scripts/verify-ui.py`. **0.25 h** per batch.
13. **Windows Update UI** (`update.js`) for goal 2. These are new strings: write them nl+en in the page's `makeT` from the
    start, as H's `notes/windows-update-design.md` §0/§6 plans. Re-target the host half from Python/TSV to Rust. That
    work belongs to the update mapping and is not counted here.

## Risks

* **Dutch drift**:
  * through the E1 port: the endonym, and plural forms for a count of 1;
  * through the GNOME wording fixes.

  Without a recorded nl baseline **before** step 2, there is nothing to prove "Dutch unchanged" against.
* **Codex's comparison tests.** Codex's own tests outside the repo (`../../work/native-energy-test`, the reference-UI
  comparisons in VALIDATION_*.md) may assert today's strings or the original UI files. Changed vendor files can make them
  fail, or make "same as the Python reference" claims stale.
* **Mixed language from the host.** The host language follows `settings.language` at call time, not the caller. A
  `?lang=en` test window against a Dutch host therefore shows mixed text. If X-Shell-Lang is adopted later, the backend
  proxy (`runtime.rs:391-402`) has to forward it.
* **System-locale text stays as it is.** Some text follows the process or system locale whatever the setting:
  `gio::content_type_get_description` fallbacks, PipeWire sink names, NetworkManager connection names, PackageKit
  messages. On Robin's en_US machine, nl mode keeps showing some English there. This is not new.
* **Cached host text.** Storage categories (`storage.categoriesReady`) and other cached host results may stay in the old
  language until they are fetched again. I did not check this.
* **Branding wording.** "Win11 Shell" appears on about 50 source lines in scope (for example `time.js:28`
  "Weergavetaal van Win11 Shell"), while the product is "Win11 voor GNOME" (README). This is a separate wording decision
  and should not be mixed into the English work.

## Not checked

* I did not run the UI in a browser or in WebKit, and took no screenshots. The only image I looked at is Codex's
  `work/native-energy-test/energy-en.png`.
* Host counts (`ApiError::new` sites, Dutch-looking lines) come from grep. I did not walk every error path, and I did
  not check whether each Dutch-only message can actually reach a Settings page on GNOME.
* Other UI areas were not checked: shell, search.js (Codex rebuilds the settings index on a language change), quick
  settings, notification centre, Explorer, Taakbeheer, Alles, mocks, `core/software.js` beyond counting.
* GNOME extension strings, native window titles (`host/src/window.rs:187-203` starts in English, then follows
  `setTitle`), desktop entries and install scripts were not checked.
* English wording was not compared key by key with Windows 11 en-GB. I only did spot checks: "Personalisation",
  "Colours", "Power & battery".
* I did not check which Settings pages are 'unsupported' on GNOME, apart from Clipboard, Display and Night light.
* I did not check whether H's i18n test harness runs against G as it is.
* `host/src/window_state.rs` (saved `normal_rect` per window kind, used in `window.rs:205`) looks relevant to goal 3
  (remembering window sizes), but I did not analyse it.
