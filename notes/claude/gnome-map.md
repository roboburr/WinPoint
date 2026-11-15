# GNOME version map: synthesis of the eight read-only reports (2026-11-05)

G = `/home/robin/Documents/Codex/2026-11-01/chat/outputs/win11-gnome` (branch `claude/gnome-english-update` = `3b898e6`;
the only untracked path is `notes/`). H = `/srv/codex-claude/claude-scratch/c2c10c4c-ae6d-4f6a-a907-342de497d54f/931db6b9-2976-4e14-8e5a-041952887025/scratch-2026-10-25-a089b6`.
W = `/home/robin/Documents/Codex/2026-11-01/chat/work`. Sources: `notes/claude/map/{ui-shell,ui-settings,ui-apps,rust-services,rust-apps,extension-packaging,updates,port-path}.md`
(all 8 present), plus my own spot-checks (section 7.1). Nothing in G or H was changed apart from this file. Nothing was built, run or launched.
Paths without a prefix are relative to G.

**Note 2026-11-14 (D-G73).** This map is the record of G at `3b898e6` and stays as written. Since then the WebKit
apps left (D-G70, D-G72; commit `6ebf169f`), and `vendor/host`, the Python reference of 1.8, 6 and 7.3, was deleted:
no installer requires or ships it, and `git show 6ebf169f:vendor/host/<path>` still shows it. Windows Update runs
no Python of WinPoint's own either (it still starts Ubuntu's check-new-release and do-release-upgrade, which are
Python). `plan.md` has the current decisions.

## 1. What the GNOME version is

1. **Processes.** One Rust crate `host/` (0.7.0, wry 0.55, webkit2gtk 4.1, gtk 0.18; 55,655 lines in `host/src`, not the ~33k the task brief says) builds 12 binaries (`scripts/install.py:17`): `win11-panel`, `win11-desktop`, `win11-start`, `win11-explorer`, `win11-app --app settings|taskmgr|alles|disks`, `win11-default-apps`, `win11-installer`, `win11-file-portal`, `win11-launch` and the index, image and software workers.
2. **Each GUI process** runs its own `tiny_http` server on `127.0.0.1:0` (`host/src/runtime.rs:203`). It serves the vendored UI (`vendor/ui`, 176 files) and `POST /api/call/<method>` with an `X-Shell-Token` header, plus SSE `/api/events`. The WebView URL is `?token&surface=shell|desktop|app[&app=…]`. There is no language in the URL (`host/src/lib.rs:373-425`).
3. **Placement.** The processes are re-executed into `app-win11shell.slice`, which has MemoryMax 1000 MiB (`host/src/process_scope.rs:17-20`).
4. **GNOME Shell 50 extension** (`extension/`, `shell-version ["50"]`, `metadata.json:5-7`):
   * it starts the panel and the desktop (`extension.js:167-169`);
   * it draws a fallback St taskbar while the panel is missing;
   * it carries out window actions and geometry, mirrors notifications, owns the shortcuts and draws the frosted strip.
   * The Rust side calls it over D-Bus through `gnome::call` (`host/src/gnome.rs:17-27`).
5. **IPC between processes.**
   * Events travel as Unix datagrams, filtered by a prefix allowlist (`host/src/events.rs:160-169`, `:333-355`).
   * Settings live in one file, `~/.config/win11-gnome/settings.json`, with one `SettingsRuntime` per process (`host/src/settings_runtime.rs:40-42`).
   * A change sends an invalidation datagram, and a 150 ms disk poll backs it up (`settings_runtime.rs:65-89`).
6. **Where the strings live.**
   * UI: 72 `makeT` dictionaries in `vendor/ui/js`, nl and en, with 2,638 keys at parity.
   * Rust host: mostly Dutch literals, plus 4 JSON `_STR` tables and inline pairs.
   * Extension: inline Dutch.
   * Installer: Dutch prints and generated `.desktop` files.
   * Docs: Dutch.
7. **Language flow.**
   * `settings.language` defaults to `"nl"` (`host/src/settings-defaults.json:3`). Settings › Tijd en taal › Taal en regio writes it (`vendor/ui/js/apps/settings/pages/time.js:120-124`).
   * Every peer re-reads the file and emits `settings.changed`. `theme.js:319-321` then calls `setLang()`, and the `onLangChange` listeners re-render.
   * The Rust host reads the language from disk on every call (`host/src/services/mod.rs:62-69`; `settings.rs:300-326` does an `fs::read` on each snapshot).
   * The extension reads settings once, in `enable()` (`extension.js:49`).
8. **Python backend.** `vendor/host` (4.1 MB) is a test reference only (`work/native-migration-state.json:6` `"nativeDefaultNoPython": true`), but `install.py:51-53` still requires it.
9. **Install** is user-local (`~/.local`, no root step). Window state is kept in `~/.config/win11-gnome/window-state.json` for the 7 own apps.

## 2. English option

### 2.1 String inventory by layer

| Layer | Already bilingual | Dutch-only, user-visible | English-only, user-visible | Source |
|---|---|---|---|---|
| UI JS (`vendor/ui/js`) | 72 `makeT` dicts, 2,638 keys, 100 % nl/en parity (by area: shell 17/553, Settings 31/790, apps 23/1,290); 116 registry/theme `{nl,en}` pairs; the language picker exists | about 20 sites. In the base (H's E1 fixes them): `core/bridge.js` 9, `shell/startmenu.js:132` `'Wissen'`, `settings/pages/update.js:159` file name, the Alles chip `'dm:vandaag'` (`alles/main.js:384`), the Alles tip examples (`panes.js:85-91`), `'Afbeelding'` (`alles/i18n.js:258`). Added by Codex (new keys needed): `alles/main.js:421-423`, `explorer/main.js:368`, `explorer/tabtransfer.js:7` (2 texts), `explorer/picker.js:49`. Mock mode only: 115 `err()` sites | `components.js:1426` `'Breadcrumb'`; 15 Codex `Error` texts (`tabtransfer.js` 12, `explorer/main.js` 3) that reach the Dutch infobar; "Everything" in the Alles note | ui-shell, ui-settings, ui-apps, port-path |
| Rust host, `services/` + core | about 262 catalog keys (`_STR`: disks 104, storage 50, sysmon 63, system 33) + 12 Wi-Fi pairs + 63 `_FRIENDLY` + about 63 software pairs + about 10 ternaries | about 232 sites (about 186 unique), mostly `ApiError` messages; plus about 42 start-up/internal texts, 25 systemd unit names (Dutch-only by design), 7 logs | none | rust-services |
| Rust host, the rest (`files/`, `apps/`, picker, transfers, gnome, launch, …) | 23 inline pairs + `files/types.json` 47 MIME pairs + `apps/catalog-data.json` (7 own apps × name/generic/comment) | about 231 sites (files 133, apps 28, process_scope 15, picker 14, transfers 13, gnome 10, wallpaper 7, launch/lib 11) | 11 GTK window titles (`window.rs:187-204`) | rust-apps |
| Rust host, all of it | – | `ApiError::new`: 261 sites in total, 251 outside `#[cfg(test)]` (my count); 203 of them carry a literal Dutch message (port-path) | – | 7.1 |
| Extension | 3 pairs (the test notification, `notificationsBridge.js:214-220`) | 15: 9 UI/a11y strings (`extension.js:66,73,108,121,234,275`; `shortcutBridge.js:161`, …), the date format `%d-%m-%Y` (`:387`), 2 errors, 1 fallback app name, 2 `metadata.json` fields | about 32 technical errors (29 `failure()` calls in `panelBridge.js`) shown in toasts; 6 fallback app names incl. `'Everything'` (`panelBridge.js:387`); 5 schema texts | extension-packaging |
| Installer and scripts | 0 | 37 messages in 7 files (install.py 13, icon_bundle.py 10, file_portal.py 6, uninstall.py 3, build.sh 2, verify-ui.py 2, package.py 1); 10 `.desktop` `Name=` + 1 `Comment=` (`install.py:111-127`), no `[nl]`/`[en]` keys | 0 | extension-packaging |
| Docs | 0 | README.md 1,622 words; RELEASE_PLAN.md 231; PACKAGING_VALIDATION.md 1,734; 25 VALIDATION*.md 16,225 words (developer reports) | – | my `wc -w` |

Combined host figure: the services map and the rest-of-host map cover disjoint files, so together they give
**≈ 463 user-visible Dutch-only Rust literal sites**. For **≈ 308** of them an identical nl row already exists in H's
`i18n/host/*.tsv`, which means the English is already written: 196 in `services/` and 112 elsewhere. About 94 of the
196 come from H's round 2, which was neither reviewed nor tested (`H/notes/english-option-design.md:2249-2254`). New
English is needed for **≈ 160** texts (≈ 40 in services and ≈ 119 elsewhere, the latter covering all of picker,
transfers, process_scope, gnome, wallpaper and `system/energy.rs`). Counted the other way: 507 of H's 588 rows occur
verbatim in G (port-path F5).

**GNOME-wrong wording in both languages** (fixing it changes Dutch, so Robin decides):
* the Clipboard page tells the user to pick the "Win11 Shell" session (`clipboard.js:23/41`), because `clipboard.list` is always `supported:false` (`host/src/services/shell/clipboard.rs:30`);
* the Display and Night light notes (`display.js:26/38/62/74`);
* `system/power.rs:154` "…Hyprland en hyprsunset nodig";
* `services/display.rs:86` "alleen in de Win11-sessie".

### 2.2 What already works in English

* The UI switches live in every process. A switch made in Settings reaches the panel, Start, the desktop and all apps within about 150 ms.
* The host follows the new language on the very next call.
* The app catalogue polls the language every 150 ms and emits `apps.changed`, so Start, the taskbar and search get the `Name[en]` names (`host/src/apps/mod.rs:221-244`). Codex validated this in both languages (`VALIDATION_APPS_CATALOG.md:7,24`).
* Already bilingual in the host: disks, storage, sysmon names, Bluetooth, Wi-Fi failures, places, MIME names, new-item names, the "- kopie"/"- Copy" suffix, the software plans and the test notification.
* 31 of Codex's harnesses seeded `"language":"en"`. Nothing asserted the English text, though, and `W/desktop-language-test/checks.py` was never run (extension-packaging (b)).

### 2.3 Plumbing that must be built

1. **UI core.** Take H's `core/i18n.js` (65 lines → 153; adds `countOf`, `list`, `collator`, `i18nReport`) and H's `core/format.js` (cached `Intl`). Merge `core/bridge.js` E1: the bridge's own texts into `makeT`, `e.key`/`e.params`, and optionally `X-Shell-Lang`.
2. **Re-fetch host text after a switch.** These parts are in H's E1:
   * Quick settings: `refreshAll()`.
   * Schijven: `load()` after a switch.
   * Taakbeheer: `store.langChanged()`.
   * Explorer and Alles: keep state over the reload; Explorer through `restoreAfterReload()`, which runs before ticket and session.
   * Disks and Taakbeheer: drop in-flight calls made in the old language.
3. **Rust catalogue and error envelope.**
   * Add `host/src/i18n.rs` (section 2.4).
   * `ApiError` gets an optional `key`/`params`, added next to `code`/`message`.
   * Contexts without a `ServiceContext` (picker, transfers, gnome, launch, process_scope, wallpaper) need a cached settings reader that checks the mtime.
4. **Text frozen in one language.** Each of these needs its own fix:
   * the `index.status` message and root errors (`index.rs:255`, `index/engine.rs:161-165`): re-send after a switch;
   * software job messages written into the job file (`software.rs:471-615`);
   * notifications: `appName "Melding"` is stored in every item (`notifications.rs:361`), so store `""`;
   * caches that do not key on the language: disks 1 s, `storage.volumes` 2 s.
5. **Extension.**
   * Read `settings.language` from the file it already reads; a missing or unknown value means nl.
   * Add `Gio.File.monitor_directory` with `WATCH_MOVES`, because the store writes by rename.
   * Re-label the fallback bar, accessible names, the Ctrl+Alt+Tab group (`removeGroup`/`addGroup`) and the clock format.
   * Do **not** use the GNOME locale: gnome-shell runs `LANG=en_US.UTF-8` while Win11 defaults to nl.
   * The ~32 English bridge errors: map them by `error.code` on the host or UI side.
6. **GTK window titles.**
   * Give each kind a title in nl and en.
   * Mirror `document.title`. The host already handles a `setTitle` message (`window.rs:463-466`), but `appframe.js:255` never sends it in a native window. Fix that, or use wry's title-changed handler.
   * This is what makes taskbar previews, Task View and the GNOME overview follow the language.
7. **Installer.**
   * Add `--lang`. The first install guesses from the locale; an existing install never changes (H D4/D13, H's `detect_info()`/`dutch_signal()`).
   * `.desktop` files get `Name[nl]`/`Name[en]`/`Comment[..]`.
   * Print bilingual messages.
8. **Things that cannot switch live** (document them, don't fight them): GNOME's own surfaces showing `.desktop` `Name=`, the `metadata.json` name, polkit prompts (GNOME Shell's agent uses en_US here), and PackageKit, PipeWire and NetworkManager texts in the system locale.
9. **`X-Shell-Lang` is optional.** The language-switch race H's D15 fixed does not exist in G (section 7.1, check 1). The header only stops a `?lang=en` test window from getting Dutch host text. If added, also forward it in the backend proxy (`runtime.rs:391-402`).
10. **Never pass `?lang=` from Rust.** It *forces* the language (`i18n.js:14-17`, `theme.js:50`) and breaks the live switch.

### 2.4 Recommended mechanism

* **UI: keep `makeT`.** It already holds 2,638 keys next to their code. Port H's `core/i18n.js`. Plurals are `{one, other}` (and `zero?`), and `countOf` prefers a numeric `count` over a formatted `n`.
* **Rust: H's TSV format as it is.** Put the catalogues under `G/i18n/host/<area>.tsv` (`key<TAB>nl<TAB>en`, escapes `\n`/`\t`, `{name}` placeholders, `key.one/.other/.zero`, first key segment = file stem).
  * Embed them with `include_str!`, so there is no missing-file path at runtime.
  * `host/src/i18n.rs` offers `Lang`, `tr`/`trf`/`trn`, `decimal`, `join`, and `ApiError::tr(code, lang, key, params)`.
  * Fallback: current language → nl → key, each miss logged once.
  * The language comes from `ctx.language()`, so all existing call sites keep working.
  * Convert the 4 `_STR` JSON tables; their nl equals H's TSV nl 100 %.
  * Keep the complete inline pairs (software `t()`, `_FRIENDLY`) and mark them for the checker.
* **Extension and Python installer: inline `{nl,en}` tables** (H D14). They are small, and they must work on failure paths.
* **Shared between layers:** the conventions (semantic keys, `{name}`, plural rule n == 1, fallback to nl) and **one checker** (H's `check.py`, re-rooted).
  * Do **not** make Rust and JS share one catalogue file. The host returns text already rendered, and the UI dictionaries are module-local (rust-services, proposal item 7).
  * New Windows Update texts should use the same carriers from the start: `makeT` in `update.js` and `i18n/host/update.tsv` in Rust. They should not be inline pairs, which is what updates.md assumed.
* **Dutch stays byte-identical.** Always take the nl column from G's Rust literals, not from H. Freeze it with a frozen-nl snapshot and a UI dump baseline.

### 2.5 Port path for H's UI work (merge results)

* **Base.** G vendored the 2026-10-27 interim UI: 161/161 hashes match `H/dist/win11shell-2026-10-27-interim*.tar.gz`.
* **H's side.** H changed 44 JS files and no CSS or assets, so the Windows look is not at risk:
  * 3 are E0a;
  * 33 are E1;
  * 4 are E1 plus the E-review fixes;
  * 1 holds only services-split code;
  * 3 mix split code and E1.
* **H is frozen.** H's UI has not changed since 09:42 (verified: the newest files in `H/ui` are from 09:40-09:42), and H is frozen (`english-option-design.md:2249`).
* **Trial result** (`notes/claude/map/merge-trial/`): 56 files changed on one or both sides. 55 merge cleanly and 1 conflicts: `js/desktop/desktop.js`, one hunk at `onLangChange`. Resolve it as `if (loaded) { refreshView(); reloadSoon(); }`.
  * `out-nosplit/` re-merges the 3 mixed Taskmgr files on the 11-04 tarball, which drops the split code; `hostbanner.js` stays as G has it.
  * `proposed/` holds 43 ready files. All 133 modules pass `node --check`. `check.py` gives 0 UI errors and 52 glossary warnings, the same as H.
* **Steps, in this order:**
  1. Port H's dump harness to `vendor/ui` and add mocks for the 18 G-only methods. Record **G's own nl baseline at `3b898e6` first**; H's `baseline-nl.txt` cannot be reused.
  2. Apply `proposed/`.
  3. Move the 6 Codex Dutch-only texts into keys: the Alles `firstNoteNative` (English says "Alles", not "Everything"), the Explorer rename error, 2 `tabtransfer` texts, and the picker `saveAs` key (this removes the `=== 'Opslaan'` coupling). Then fix the second en "Power" heading (`power.js:78`).
  4. Regenerate `vendor/ui-sha256.json` (no script exists yet: `verify-ui.py:9-19` only checks) and add an `english_option` entry to `vendor/ui-patches.json`. `package.py:17` refuses to package otherwise.
  5. Diff the nl dump against the baseline; only rows in G's `BASELINE-CHANGES` may differ. Run the en dump with `?i18ncheck=1` and the ratchet: 84 H keeps, plus `shell.js:67`.
  6. Live-switch test in a private headless GNOME Shell (section 5). Cover Explorer with tabs, tab transfer and the picker; Alles while indexing; Taakbeheer; the desktop; quick-settings Wi-Fi; and the hidden preloaded Start.
* **Dutch changes that come with E0a/E1** (Robin must accept them or they are held back):
  * "Nederlands" → "Nederlands (Nederland)" (`time.js:122`);
  * singulars for a count of 1, from 14 plural objects plus `about.js`/`display.js` ("1 proces", "1 kern", "1 item kon niet …");
  * the formatted-1000 plural ("1.000 items");
  * "Afbeelding";
  * the Alles `hintHidden` and `trashFailedBody` texts.

### 2.6 Reusable H tooling

* **Reusable as they are, after changing the UI root (`ROOT/ui/js` → `vendor/ui/js`):**
  * `tools/i18n/{jsdicts,tok,scan_ui,pairs,analyze}.mjs`;
  * the UI and TSV rules of `check.py`;
  * the `ui` part of `ratchet.py`;
  * `glossary.tsv` (101 rows, Windows 11 en-GB) and `glossary.py`;
  * `scan_en.py`, `corpus.py`, `dutch_words.py`;
  * `tests/i18n/{dutch-words.txt (1,295), allow-ui.txt (84 rows / 90 lines), core_i18n.test.mjs, intl_jsc.py}`;
  * the dump harness (`run.sh`, `serve.py`, `dump.html`, `freeze.js`, `blocks.py`, `surfaces.json` with 134 surfaces).
* **Sites to re-root:** `check.py:359,385,391,955`, `scan_ui.mjs:96,103`, `corpus.py:82,89`, `glossary.py:115,129`, `audit.py:49,218-219`, `ratchet.py:278`, `intl_jsc.py:27`, `serve.py:45`.
* **Catalogues:** `i18n/host/*.tsv` has 21 files and 588 rows. Reuse the en column of the 16 stems that exist in Rust (528 rows), and drop `ctl`, `main`, `surfaces`, `windows` and the Python `core` rows.
* **Not reusable:** `host/i18n.py` (port its design, not its code), `scan_py.py`/`extract_py.py`, `scan_sh.py`/`extract_sh.py`, the bash catalogues (`i18n/{common,install,uninstall,session}.tsv`, `lib.sh`), the transcripts and H's `nl-frozen/`.
* **New for G:** a Rust literal scanner for the ratchet, and an allowlist for host input words (`index/query.rs:273-847`), the CSS markers (`settings_effects.rs:17/20`) and logs.

### 2.7 Effort (agent-hours; the reports overlap, so this de-duplicates them)

| Part | Hours | Basis |
|---|---|---|
| UI port, the Codex text fixes and manifests | 6-8 | port-path W1-W3; ui-apps U0-U6; ui-settings 1-5 |
| Tooling, nl baseline and dumps | 12-15 | port-path W5-W8 |
| Rust i18n core, error envelope and optional header | 5-6 | rust-services 1-3; rust-apps 1 (counted once) |
| Rust text conversion (≈ 463 sites, ≈ 308 with H English) plus frozen-language fixes | 30-38 | rust-services 4-7 (≈ 19); rust-apps 2, 6-8 (≈ 9-12); ui-apps R3-R8 is a subset |
| Rust ratchet, nl freeze and English review of round-2 and new rows | 6-8 | rust-services 8-10; rust-apps 3 |
| Window titles and the extension (labels, monitor, error codes) | 6-9 | rust-apps 4, 6; extension-packaging 1-3 |
| Installer, desktop entries and `--lang` | 5-7 | extension-packaging 4-5; port-path W13 |
| Private GNOME harness, live-switch tests and the nl/en screenshot pass | 12-16 | extension-packaging 6-8; port-path W4; ui-settings 11 |
| README.en and glossary lock-in | 5-6 | port-path W14 |
| **Total** | **≈ 87-113** | port-path's 52-58 h undercounts the host (31-36 h against 45-50 h from the two host maps) and leaves out the harness |

## 3. Windows Update

### 3.1 What exists in G

* **Update count only.**
  * `system.updates` runs `apt list --upgradable` with a 30-minute TTL (`host/src/services/system/info.rs:289,306`).
  * "Controleren" only re-reads that list and never refreshes the package lists (`update.js:122`).
  * "Software-updater openen" tries `update-manager`, then `gnome-software`, then `snap-store` (`system/power.rs:100-119`; the message at `:116` is Dutch only).
* **The page** has a read-only hero (`vendor/ui/js/apps/settings/pages/update.js:93-137`, `makeT` nl/en at 12-91). Home, System, About, Taakbeheer and the sysmon health item use the count.
* **A PackageKit client exists for software install/remove** (`host/src/services/software/packagekit.rs`, 309 lines):
  * CreateTransaction, `SetHints` with `interactive=true` and a hard-coded `locale=C.UTF-8` (`:135`), simulate then run, the signals, Percentage/Status/AllowCancel and Cancel;
  * a 1-second `GetAll` inside a 20 ms sleep loop (`:251`) and a 2-hour cap.
  * Job, plan and lock files with owner pid + start time (`software.rs:91-109`); no reattach.
  * `Software::shutdown` joins the worker threads (`software.rs:777-785`).
* **Precedents to reuse:** pkexec (`storage/cleanup.rs:661-735`), a terminal launch (`files/actions.rs:611-672`), `launch::spawn` role `app` (outside the 1000 MiB slice), and a fake PackageKit (`W/native-software-test/suite.rs:41-56`).
* **Missing entirely:** apply, progress for updates, `reboot-required`, release upgrade, Prompt (LTS/normal). grep finds no `reboot-required`, `RequireRestart`, `do-release-upgrade` or `meta-release` in `host/src`, `vendor/ui/js` or `extension` (verified).

### 3.2 What carries over from H's design (`H/notes/windows-update-design.md`)

These carry over unchanged:
* the PackageKit flow: RefreshCache → GetUpdates → simulate `UpdatePackages(SIMULATE|ONLY_TRUSTED)` → real;
* all hero, row and card states with their nl/en texts (§3.1-3.8);
* snapshot and job files with the boot id, reattach, foreign transactions, cancel only while `AllowCancel` (§4.4-4.6);
* the reboot-required FileMonitor (§4.7.1);
* no offline updates; ids only from the current snapshot; snaps read-only (§4.9-4.13);
* release detection from Ubuntu's own data (§5.1);
* the apt-tree compatibility check with its own `APT_CONFIG` (§5.2);
* the Prompt helper and policy (§5.3);
* `pkexec do-release-upgrade` under screen in a terminal, with temporary linger and the attach helper (§5.4/5.6);
* Robin-only actions R1-R6, of which R5, the upgrade test in a VM, stays essential.

### 3.3 What changes for GNOME (updates.md F3)

* **Polkit agent.** GNOME Shell's built-in agent replaces hyprpolkitagent, so drop `noAgent`.
  * Updates need no password for Robin: `org.freedesktop.packagekit.rules` allows active sudo members.
  * A release upgrade always asks.
* **Sign-out trigger.** It is a running gnome-shell that maps a deleted `/usr/bin/gnome-shell` or `libmutter-*`.
* **Restart after library updates.** A WebKitGTK, GTK or libsoup update means a whole-UI restart, and `shell.restart` restarts only the caller (`services/shell.rs:216-224`, gap X1).
* **Idle inhibitor.** Not needed (GNOME's lock screen is gnome-shell itself). Keep a power guard, because the logout path uses `--no-prompt` (`power.rs:44-49`).
* **update-notifier really runs here,** and update-manager opens by itself every 7 days. See decision D2.
* **Compatibility check.** The blocking list becomes the packages behind our NEEDED sonames plus `libarchive13t64`.
  * **The decisive test is the gnome-shell candidate major against `metadata.json` `shell-version`.**
  * `releases.json` should record the GNOME major per codename.
* **Ubuntu's GTK upgrader** is now a legitimate option (D1).
* **Process model.** Run transactions in the Settings process, with the job files as shared state, and do not join threads on shutdown.
* **Post-upgrade check.** It must be an XDG autostart entry outside the extension, because the extension is exactly what stops loading on a new GNOME major.
* **Placement.** The upgrade terminal must go through `launch::spawn` (`app.slice`), never `app-win11shell.slice`, where MemoryMax 1000 MiB could OOM-kill dpkg.
* **No root install step exists.** The 2 policies and 2 helpers need a pkexec step or the final package (D4).
* **No longer applies:** the Python thread/GIL design, the S7 gate, win11-csd, hyprland.lock, hypridle/hyprlock, `_safety.py` and nested-Hyprland screenshots.

**Machine state** (read by the updates agent; I did not re-check it):
* `/run/reboot-required` is set: gnome-shell went from 50.1 1.4 to 1.5 at 06:54 today.
* 26.10 was released on 2026-10-15 but is not offered yet; the meta-release caches have no "stonking" entry, and `Prompt=normal`.
* `ubuntu-dock` is disabled.

### 3.4 Gap list (updates.md, agent-hours)

| # | Item | h |
|---|---|---|
| G-W1 | Reusable fake PackageKit (properties, signals, timelines) on a minimal bus config, with `NO_AT_BRIDGE=1` | 7 |
| G-W2 | Rust `update.*` service: refactor `packagekit.rs` to event callbacks, ONLY_TRUSTED, locale from `ctx.language()`, no 2 h cap, reattach, reboot FileMonitor, deleted-mapping scan, power guard, the `events.rs` prefix `update.` (missing from `:333-355`), `api-methods.json`, dry-run | 16 |
| G-W3 | The page: hero, rows with ProgressBar, restart InfoBars, mocks, about 20 states × nl/en screenshots | 13 |
| G-W4 | History, advanced options, the update-notifier setting (D2) | 7 |
| G-X1 | Restart all Win11 processes after a WebKitGTK/GTK/libsoup update | 3 |
| G-W5 | Release detection and Prompt (LTS/normal) helper and policy, plus a root install step | 11 |
| G-W6 | Compatibility check and the "Ubuntu 26.10 is beschikbaar" card (GNOME major against `shell-version`) | 10 (cheap variant 6) |
| G-W7 | Start the upgrade (terminal via `launch::spawn`, linger), attach, and the post-upgrade autostart check | 11 |
| G-W8 | Review, docs and screenshots | 6 |
| G-W9 (optional) | Start power badge "Bijwerken en opnieuw opstarten", fwupd row | 6 |
| G-X2 (precondition for an "ok" verdict) | Port and test the extension on 26.10's GNOME major | 4-16 |

Totals: Phase A (W1-W4 + X1) ≈ 46 h; Phase B (W5-W7) ≈ 32 h; overall ≈ 84 h, plus W9 and X2.

## 4. Window size memory

**What exists** (rust-apps (c); verified at `window_state.rs:21-26,318,339,405`):
* **Apps covered:** 7 own kinds (explorer, settings, taskmgr, alles, disks, defaultapps, installer). The extension accepts the same ids (`panelBridge.js:505,518`).
* **Store:** `~/.config/win11-gnome/window-state.json` with `{version:1, apps:{kind:{normalRect, maximized, session}}}`. Written with mode 0600, flock, temp file + rename and a merge that keeps other kinds.
* **Restore:** the GTK default size comes from the saved w/h (`window.rs:205-258`) with a minimum of 640×480. Then maximize if needed. After `show_all`, the worker sends `windows.hostRestore` up to 12 × 500 ms. The extension then clamps the rect to the work area and calls `move_resize_frame` after the first frame (`panelBridge.js:529-565`).
* **In use:** Robin's file has entries for explorer, settings and taskmgr.

**What is missing:**
1. **Saving is fragile.** The file is written only in `flush()`, at clean exit or on Drop (`lib.rs:471-473`). There is no SIGTERM/SIGHUP handler (only `PR_SET_PDEATHSIG`, `lib.rs:536`), so a window still open at logout loses its geometry.
2. **The 500 ms poll.** Every open own-app window polls gnome-shell over D-Bus every 500 ms (`window_state.rs:318`). It should be replaced by a push.
3. **No cascade.** A second window opens exactly on top of the first, and the last window to close wins.
4. **Third-party apps are not covered at all.** That needs an extension feature:
   * track by app id and save the normal rect and maximized state, debounced and on `unmanaging`;
   * restore at first frame;
   * skip dialogs, transients and fixed-size windows;
   * an opt-out list for apps that restore themselves;
   * a capped store;
   * a Settings toggle with nl/en text.
5. **Picker size** is not remembered (Windows remembers the file dialog size).

**Cost:** own-app hardening 3-4 h; all apps 9-13 h (only if Robin decides).

**Note:** Codex's deploy records call the extension-side restore "windowGeometry" (`W/native-live-update/deployed.json`). Changes to it need a re-login.

## 5. Build, test and package

**Build** (memory: about 3.7 GB free, so use 2 jobs):
```bash
cd /home/robin/Documents/Codex/2026-11-01/chat/outputs/win11-gnome
WIN11_BUILD_JOBS=2 ./scripts/build.sh          # cargo build --release --locked -j2 (scripts/build.sh:13); output target/release
python3 -B scripts/verify-ui.py                # after any vendor/ui edit; first regenerate vendor/ui-sha256.json (needs a new small script)
```
`target/` holds only `debug/` today, and there is no `target/release`. Whether every crate is in the local cargo cache
for an offline `--locked` build was not checked.

**Tests.**
* **Rust unit tests.**
  * There are 256 `#[test]`; 8 are `#[ignore]` and need Codex's `W/native-*-test` private harnesses (`private_bus_tests.rs:12`).
  * Three oracle files, `services/{system,shell,disks}/reference-tests.json`, assert Dutch messages, so they guard "Dutch unchanged".
  * Run them as `cargo test --locked -j2`, inside `dbus-run-session` with a private HOME and XDG dirs, `GSETTINGS_BACKEND=keyfile` and `NO_AT_BRIDGE=1`. I have not checked whether any non-ignored test talks to the session bus.
* **UI.** Use H's dump harness in mock mode, from a static server on `vendor/ui`, one surface at a time (about 1 GB). Use `check.py` and the ratchet once they are ported.
* **GUI and extension (the safe method, never the live display).** This is a private headless GNOME Shell, following Codex's `W/desktop-bridge-test-0.7.0/run-combined.py` and `W/settings-native-test/run.py`, with the guards from extension-packaging (d):
  * `dbus-run-session -- env -u DISPLAY -u WAYLAND_DISPLAY …`;
  * `NO_AT_BRIDGE=1 GSETTINGS_BACKEND=keyfile`;
  * private `HOME`, `XDG_{CONFIG,DATA,CACHE}_HOME`, and `XDG_RUNTIME_DIR` (0700);
  * `XDG_DATA_DIRS=/usr/local/share:/usr/share`;
  * `WIN11_INSTALL_PREFIX`, `WIN11_BUILD_DIR`;
  * install with `scripts/install.sh` into that prefix;
  * a private extension copy patched with `unsafe_mode`;
  * `gnome-shell --headless --virtual-monitor=1440x900 --wayland-display=<name> --no-x11`;
  * drive it with `org.gnome.Shell.Eval`, `WIN11_DEV_COMMAND`/`WIN11_DEV_REPORT` and Clutter virtual input;
  * take screenshots with `org.gnome.Shell.Screenshot` on the private bus;
  * test the language switch by rewriting the private `settings.json`.
* **Harness rules.**
  * The harness must abort if it sees `/run/user/1000/bus` or `XDG_RUNTIME_DIR=/run/user/1000`.
  * Keep it inside the project (`tests/gnome/`), not in `/tmp`: memory note "Scratchpad lost on reboot".
  * Signal only its own children.
  * Never run `uninstall.sh` outside it: even `--files-only` writes GSettings (`uninstall.py:21-31`).

**What needs a GNOME re-login:**
* the first install;
* every change to `extension/*.js`, `stylesheet.css`, `metadata.json` or the schema. That covers the extension's English labels, all-app window memory and any extension port to GNOME 51;
* the portal backend selection.

New binaries and UI files need the project's own processes restarted. We may not signal processes, so for the live
PC that means Robin logs out and in. Apps pick up a new build the next time they open; `.desktop` changes only need
`update-desktop-database`.

**Packaging.**
* `./scripts/package.sh` → `package.py`:
  * it runs `verify-ui.py` (`:17`) and `icon_bundle.validate`;
  * it requires all 12 binaries in `target/release`;
  * it writes `dist/win11-gnome-<ver>-linux-x86_64.tar.gz` and `.sha256`.
* `dist/` holds only 0.2.0-0.6.1. No 0.7.0 archive exists, although `PACKAGING_VALIDATION.md:7-11` describes one.
* Expect a much larger archive: the icon tree is 81 MB.
* The report list in `package.py` names a missing `VALIDATION_PROCESS_MEMORY.md` and leaves out `VALIDATION_RESOURCES.md`.
* Bump the version in `host/Cargo.toml`, the `metadata.json` `version`/`version-name` and the README.

**This PC.**
1. First back up `~/.local/bin/win11-*` and `~/.local/share/win11-gnome`. The live `win11-panel` (28,363,968 bytes, mtime 2026-11-04 23:32, sha `17437f50…`) matches no Codex record.
2. Run `WIN11_BUILD_DIR=$G/target/release ./scripts/install.sh`. The plain command fails today, because the stale `G/bin` holds only 6 binaries and `install.py:42` prefers it.
3. Robin logs out and in.

**NucBox** (H notes: GMKtec NucBox G3, Intel N100 with Mesa, 8 or 16 GB, Ubuntu 26.04 Dutch with language-pack-gnome-nl, no Rust toolchain):
1. Check the tarball `.sha256`, then unpack it.
2. `sudo apt install libwebkit2gtk-4.1-0 libgtk-3-0t64 libarchive13t64 python3-gi libglib2.0-bin desktop-file-utils`. Whether WebKitGTK 4.1 is on a stock 26.04 is not verified, and `install.py` has no runtime preflight.
3. `./scripts/install.sh`.
4. Log out and in.
5. `gnome-extensions enable win11-gnome@win11shell.local`, and optionally disable ubuntu-dock.
6. Keep the unpacked tree: the uninstaller is not installed.

## 6. Risks and open questions

**Only Robin can decide:**
1. **Accept the Dutch changes from H's E0a/E1** (section 2.5). Suggested default: accept the singular and plural fixes (they are bug fixes), and keep "Nederlands" rather than "Nederlands (Nederland)".
2. **Rewrite the GNOME-wrong Dutch texts** (Clipboard, Display, Night light, the Hyprland messages)?
3. **Language of the plain `.desktop` `Name=` keys.** GNOME's overview follows `LANG=en_US`, so English plain keys show English in GNOME even in Dutch mode. Also the `metadata.json` name and description.
4. **Does "Alles" stay "Alles" in English** (H D5)? Codex uses "Everything" in the window title (`window.rs:193`), the extension fallback (`panelBridge.js:387`) and the Alles note.
5. **Windows Update decisions D1-D6** (updates.md). The defaults are a terminal + screen + linger, update-notifier auto-open off through a toggle, hold on an untested GNOME major, a root step in the final package, no background watcher in v1, and "Opnieuw opstarten vereist" when gnome-shell was updated. Also whether to show the LTS/normal choice prominently: interim releases need a tested extension every 6 months.
6. **Window memory for all apps** (yes or no; size only, or size and position)?
7. **Branding.** "Win11 Shell" appears in about 50 Settings source lines, against the product name "Win11 voor GNOME". Keep this separate from the English work.
8. **English names for the 25 systemd units,** or keep systemd's own description.
9. **The unknown live `win11-panel` binary,** and approving the live deploy and its re-login.
10. **NucBox:** which language, and whether to clean up the old Hyprland Win11Shell install there.

**We can decide, with a sensible default:**
* the mechanism in section 2.4;
* `X-Shell-Lang`: add it for tests only;
* re-labelling the extension through a file monitor;
* mapping extension errors by code;
* the second en "Power" heading;
* localised titles plus a `document.title` sync;
* language-keyed caches;
* making the installer prefer `target/release`;
* a script to regenerate the manifest;
* `-j2` builds;
* keeping `vendor/host` until packaging is redone;
* a TSV catalogue for the update texts.

**Risks:**
* **Dutch drift** while moving ≈ 463 Rust literals. Extract them by script, freeze the nl column, and keep the Rust oracles green.
* **Unreviewed H English** (round 2).
* **English layout.** Longer text meets 14 `nowrap`/ellipsis rules in `settings.css` alone, plus Codex's real taskbar search field. No widths were measured anywhere.
* **A GNOME 51 upgrade** would leave Robin with no taskbar and no dock.
* **WebKit/libsoup updates under a running UI.**
* **Every extension test** runs only in a headless session that differs from the live one (virtual monitors, shared system bus).
* **Memory:** a release build and the dumps compete for about 3.7 GB.

## 7. Completeness critique

### 7.1 Spot-checks (load-bearing claims, checked by me against G)

| # | Claim (report) | Checked at | Result |
|---|---|---|---|
| 1 | The host reads the language fresh per call, and peers re-read the file before dispatching, so H's D15 race does not occur (ui-shell, rust-services) | `services/mod.rs:62-69`; `settings.rs:77-80, 300-326` (`fs::read` on every snapshot); `settings_runtime.rs:65-73` (`adopt_disk_exact` before the event); `events.rs:113-121` | **Held.** port-path's "D15 race applies to G" **did not hold** |
| 2 | No `X-Shell-Lang` is read anywhere in the host | `grep -rn X-Shell-Lang host/src` | **Held** (0 hits) |
| 3 | Window titles are fixed English and never follow the UI (rust-apps) | `window.rs:187-204`; `appframe.js:252-255` (sends `setTitle` only when not native); `window.rs:463-466` handler exists but receives nothing; no title-changed handler | **Held.** ui-settings' "then follows setTitle" **did not hold** |
| 4 | No Dutch first-paint flash for an English user (ui-shell) versus "first paint renders Dutch" (rust-apps) | `main.js:73-74` awaits `initTheme()` (theme.js:355-366 awaits `settings.get`, then `apply` → `setLang`) before any surface mounts | ui-shell **held**; rust-apps' flash claim **did not hold** for mounted surfaces |
| 5 | Merge trial: 1 conflict (desktop.js), Explorer and Alles merge cleanly, and the reload restore runs before ticket/session (port-path) | `merge-trial/summary.tsv` (56 rows: 55 × 0, 1 × 1); conflict markers: desktop 1, explorer 0, alles 0; `proposed/js/apps/explorer/main.js:2016` | **Held.** port-path's "57 rows" is 56 |
| 6 | `ApiError::new` sites: 261 / 251 non-test | grep, and cutting each file at `#[cfg(test)]` | **Held** |
| 7 | `install.sh` from a G checkout fails on the stale `G/bin` (extension-packaging) | `install.py:42,47-50`; `G/bin` has 6 binaries | **Held** |
| 8 | Updates: only an `apt list` count, 30-min TTL; PackageKit `locale=C.UTF-8`; 20 ms loop; no restart or release code (updates) | `info.rs:289,306`; `packagekit.rs:135,251`; grep over host/ui/extension | **Held** |
| 9 | Window memory: 7 kinds, 500 ms poll, no SIGTERM flush (rust-apps) | `window_state.rs:21-26,318`; grep SIGTERM/unix_signal (only PDEATHSIG and child kills) | **Held** |
| 10 | Extension: `shell-version ["50"]`, settings read once, Dutch labels, `'Everything'`, 29 `failure()` calls (extension-packaging) | `metadata.json`; `extension.js:20-31,49,66,73,108,121,234,275`; `panelBridge.js:387`; 30 `failure(` occurrences incl. the definition | **Held** |
| 11 | `"Melding"` stored, default `nl`, the picker, `core/i18n.js` pre-E0a (several) | `notifications.rs:361`; `settings-defaults.json:3`; `time.js:120-124`; `i18n.js` 65 lines | **Held** |
| 12 | `bridge.js` has 9 Dutch-only sites (ui-shell) versus 15 (ui-settings) | `bridge.js:65,66,133,152,155,159,163,169,238` | ui-shell **held**; ui-settings' 15 includes the 6 ternary strings |
| 13 | The event allowlist lacks `update.`; `Software::shutdown` joins its workers (updates) | `events.rs:333-355`; `software.rs:777-785` | **Held** |
| 14 | `uninstall --files-only` still writes GSettings (extension-packaging) | `uninstall.py:21-31` | **Held** |
| 15 | en on a Dutch system falls back to GIO's Dutch name (rust-apps) | `apps/catalog.rs:297-309` | **Plausible** from reading; I did not trace `translator.get` |
| 16 | `clipboard.list` is always unsupported; en "Power" twice; `'Wissen'`; Alles ternary with "Everything" | `clipboard.rs:30`; `power.js:57,78`; `startmenu.js:132`; `alles/main.js:421-423` | **Held** |
| 17 | `host/src` size | `wc -l`: 55,655 (53,581 without test files) | port-path's 55.7k **held**; the task brief's "~33k" did not |

### 7.2 Contradictions between the reports, and how I resolved them

* **D15 race.** port-path says it applies; ui-shell and rust-services say it does not. **It does not** (check 1). `X-Shell-Lang` is therefore a test convenience, not a correctness fix, and port-path W9 can drop that motivation.
* **Window titles.** ui-settings says they "follow setTitle"; rust-apps says they are fixed English. **rust-apps is right** (check 3).
* **First paint.** rust-apps proposes a `__win11shellHost.lang` hint against a Dutch flash. ui-shell shows that no surface mounts before the settings arrive. **The hint is not needed.**
* **Explorer and Alles merges.** ui-apps calls them "manual merge"; port-path calls them clean. **They are textually clean** (check 5). The semantic review that ui-apps asks for is still needed (Explorer: three restore sources; Alles: Codex's in-place progress overlay).
* **Format for the update texts.** updates.md says to paste H's texts as inline `t(lang,nl,en)` pairs; rust-services, rust-apps and port-path recommend a TSV catalogue. I recommend the catalogue (section 2.4), with i18n.rs built before G-W2.
* **Polkit.** extension-packaging says a polkit rule for password-less updates would need a root install. updates.md shows that `org.freedesktop.packagekit.rules` already allows Robin. A root step is needed only for the Prompt and attach helpers (D4).
* **Host counts are not comparable across reports:** rust-services 232 sites (services); rust-apps 231 (the rest); ui-apps 212 unique (a subset); port-path 630 literals including logs, and 203 Dutch `ApiError` messages. They are consistent once scopes are lined up.
* **Effort.** port-path gives 52-58 h for the whole English option; the per-area reports add up to about 90-110 h (section 2.7).
* **Small mismatches.**
  * `allow-ui.txt` is 90 lines (ui-shell) and 84 rows (port-path); both are true.
  * rust-apps says "windowGeometry" occurs nowhere in G or H. That is true, but it does occur in Codex's `W/…/deployed.json`.
  * picker/transfers literals are 14/13 (rust-apps) against 13/12 (ui-apps).

### 7.3 What no report covered

* **Rust tests and oracles.** No report covered how to run `cargo test` safely, the 8 ignored harness tests, or the 3 `reference-tests.json` oracles as a Dutch guard. Only rust-services mentions the system oracle.
* **No manifest generator.** No tool regenerates `vendor/ui-sha256.json`; every report assumes one exists.
* **`vendor/host`.** The Python reference (4.1 MB) is still required by `install.py:51-53` and shipped by `package.py`. Nobody asked whether to keep it, or inventoried its strings (they matter only with `WIN11_REFERENCE_INSTANCE`).
* **WebKit's own strings** (form validation, spellcheck, default context menus) follow the process `LANG` (en_US). Nobody checked whether the UI suppresses all of them.
* **Polkit and GNOME system dialogs** (password prompts, GNOME's logout dialog) are English on this PC even in Dutch mode. updates.md mentions this only in passing. It is a Win11-fidelity issue for the update flow.
* **The NucBox's current state.** It previously had H's Hyprland Win11Shell (login-loop history in `H/notes/fresh-install-requirements.md`), and the extension falls back to `~/.config/win11shell/settings.json` (`extension.js:22-23`). Migration, coexistence and clean-up of that old install were not covered. Its Dutch system locale makes the catalogue bug (check 15) relevant there.
* **English layout widths.** Nobody measured them.
* **Accessibility labels** outside the shell area were not checked.
* **Codex's harnesses in `W/*`** may pin UI hashes or Dutch strings. This was flagged as not checked in two reports and resolved in none.
* **The hidden preloaded Start** re-rendering after a switch is unverified in every report.
* **Robin's current language:** `~/.config/win11-gnome/settings.json` has `"language": "nl"` (read-only check).
* **Git.** G's `notes/` (these reports plus the merge trial) is untracked. It should be committed on `claude/gnome-english-update` before any port, so the `proposed/` set is versioned.

### 7.4 What I did not check

* I did not re-run any report's scanners, extractors or merges; I spot-checked only the claims listed in 7.1.
* I did not check the machine state that the updates agent read: dpkg, polkit rules, update-notifier, the meta-release caches and the `/proc` maps.
* I did not read Codex's `W/*` harnesses, H's `windows-update-design.md`, or the `VALIDATION*.md` contents.
* I did not check per-key English quality, or which GNOME version Ubuntu 26.10 ships.
* I did not check whether the release build works offline from the cargo cache.
* I did not check whether the non-ignored Rust tests touch the session bus.
* I did not do runtime checks of any kind.
