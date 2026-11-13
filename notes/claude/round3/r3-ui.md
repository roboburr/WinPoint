# R3-UI: English review, glossary lock-in, UI items and Windows Update A2 leftovers (2026-11-06)

Step R3-UI of round 3 (plan.md round-3 lists, D-G31, D-G41, D-G44, D-G45; cross.md §7 items 6 and 8), in the worktree
`.wt/r3-ui` (branch `r3/r3-ui`, from 18c8ffbe). Start state checked: `git status --short` empty, HEAD 18c8ffbe.
Nothing was committed, stashed or switched. No bus, PackageKit, apt, snap, gsettings, dconf, win11-* binary or
gnome-shell ran. Every Chrome run went through `tools/heavy.sh` and `tests/i18n/run.sh` or `tests/update-ui/shoot.sh`
(nonet.sh's layer). Scratch files are in `.rt/r3-ui/` (dumps, shots, the replacement script `en_fix.py` and its log).
Only files of this step's allowlist changed (list in §9).

## 1. Result (done-when; commands from W's root, `PATH=/home/robin/.local/node/bin:$PATH` for the node tools)

| Check | Command | Result |
|---|---|---|
| Checker, glossary as an error | `python3 -B tools/i18n/check.py` | **0 errors**, 72 warnings, all `rust` (check_rust's 72, unchanged; 4 of them are check_rust's own glossary findings, §4.5; corrected in the fix round, the first version said 3); **0 glossary warnings** (was 52); `glossary` is no longer in `DEFERRED` (check.py, check_ui.py); 18 context rows, none stale |
| Ratchet | `python3 -B tools/i18n/ratchet.py` | `ui: ok (89 allowed occurrences, 0 still to convert)` |
| Dictionary diff | `python3 -B tools/i18n/check_ui.py --dict-diff tests/i18n/dict-nl-3b898e6.tsv --quiet` | 2925 → 3170 rows; 9 changed, 20 removed, 265 added (+6: D-G44's keys); **0 not in BASELINE-CHANGES.ui, 0 stale** |
| nl dump | `tools/heavy.sh --tag r3-ui-dump -- tests/i18n/run.sh` | `nl dump equals baseline-nl.txt (24786 lines)` |
| en dump | `tools/heavy.sh --tag r3-ui-dump -- tests/i18n/run.sh --lang en` | `en dump equals baseline-en.txt (24783 lines)` |
| Determinism | `tools/heavy.sh --tag r3-ui-dump -- tests/i18n/run.sh --twice` and `… --twice --lang en` | `two nl dumps identical (24786 lines, sha256 9231043098614713)`; `two en dumps identical (24783 lines, sha256 ca71d5699faa149b)` |
| Record | `tools/heavy.sh --tag r3-ui-dump -- tests/i18n/run.sh --record --accept <16 ids>`; `… --record --lang en --accept <43 ids>` (the id lists: `.rt/r3-ui/accept-{nl,en}.txt`, §6) | nl: 16 changed blocks (accepted), 2 new; en: 43 changed (accepted), 2 new. The recorded files equal the scratch dumps made before (`cmp`) |
| node | `node --input-type=module --check < F` for the 49 changed modules and `tools/i18n/jsdicts.mjs` | 49/49 + 1 ok |
| node tests | `node tests/i18n/core_i18n.test.mjs`; `node --experimental-default-type=module tests/i18n/mock_settings.test.mjs` (new) | `ok 24214`; `ok 16` |
| i18n unit tests | `python3 -B -m unittest tests/i18n/test_ge0a.py`, `…/test_update_mock_rows.py`, `…/test_glossary_context.py` (new) | 21 OK (2 heavy skipped); 5 OK (1 skipped: the design-table test, update.tsv has rows); 8 OK |
| Intl | `TZ=Europe/Amsterdam python3 -B tests/i18n/intl_jsc.py` | 66 values, 0 mismatches |
| Pairs | `python3 -B tools/i18n/check_pairs.py` | 0 errors, 0 warnings (144 pairs; its glossary findings are errors now) |
| Glossary report | `python3 -B tools/i18n/glossary.py` | 124 terms, 1 with variants (Datum en tijd: GNOME's own "System › Date & Time" in time.js, right), **0 terms with pairs that lack the glossary phrase** (was 18), known deviations of §4.2: **none** (was 14 × Trash, 6 × Airplane mode) |
| Manifest | `python3 -B tools/ui-manifest.py`, `--check`; `python3 -B scripts/verify-ui.py` | regenerated (49 changed since the last manifest); up to date (181 files; 82 changed, 20 added against the original 161); OK. New patch entry `round3_english_review` (49 files + reason) |
| Screenshots | `tools/heavy.sh --tag r3-ui-shots -- tests/update-ui/shoot.sh <13 ids>`; `LANGS=en SIZE=800x1400 SUFFIX=-narrow … <3 ids>` | 26 + 3 shots, 0 failed; reviewed (§7); copies and nl/en side-by-sides in `.rt/r3-ui/shots/` |

## 2. English review

I read the whole en dump (every block: the 2,022 lines that only the en dump has, of which 860 are no dictionary
value verbatim, and the 319 shared nl/en lines with a Dutch word) and every makeT dictionary, inline pair and
registry text (3,046 rows, `.rt/r3-ui/dicts.tsv`). Every Dutch word left in the en dump is mock or host data (the
mocks answer in their own `settings.language`, nl; GNOME app names and descriptions in the system locale; Taakbeheer's
friendly process names and summary checks from the host), none is a UI string.

What I changed (en only; 175 replacements in 42 files by `.rt/r3-ui/en_fix.py`, each asserted by count, the full
before/after list is the table in §10), by kind:

* **Windows 11 contractions** (Windows' own style; the UI mixed 30 × "could not" with 15 × "couldn’t"): "could not",
  "cannot", "is not", "are not", "does not", "did not", "was not", "you are", "you will" → "couldn’t", "can’t", "isn’t",
  … in messages; kept where Windows keeps the full form ("is not responding", "Do not disturb").
* **"Please try again"** → "Try again" (the glossary's "Probeer het opnieuw → Try again"; Windows rarely says please;
  "Please wait" stays): Settings' page failure, Taakbeheer, the bridge's offline text, the main loader, the flyouts.
* **Glossary terms** (were the 52 warnings, §4): Trash → **Recycle Bin** (Taakbeheer's storage pages, 14 texts),
  Airplane mode → **Flight mode** (Quick settings, the taskbar tooltip, Netwerk en internet, the registry, search
  keywords: "flight mode" added, "airplane mode" kept as a synonym), "Bluetooth isn’t responding" → **"Bluetooth is
  not responding"** (D-G34's glossary choice, 3 texts), "Empty the Recycle Bin?" → **"Empty Recycle Bin?"** (the
  command's name, 3 dialogs), Alles' "Saving failed" → "Couldn’t save" and "Not everything could be moved" → "… to the
  Recycle Bin".
* **Dutch word order and calques**: "Better change them in Windows" → "It’s best to change them in Windows", "Better
  not use it for banking" → "It’s best not to …", "So you’re sure you chose the right volume" → "This makes sure
  you’ve chosen the right volume", "This went wrong for: {names}" → "There was a problem with: {names}", "Who may
  change this is set in the permissions" → "The permissions decide who can change this", "Alles is making an
  overview" → "building an overview", "Makes the overview from scratch" → "Builds the overview again from scratch",
  "Deleting goes to the Recycle Bin" → "Deleted files go to the Recycle Bin", "goes to the Recycle Bin" → "will be moved
  to the Recycle Bin", "Logs total {total}; everything older than {days} days goes" → "Logs take up {total};
  everything older than {days} days will be removed", "That often helps when something got stuck" → "This often helps
  when something is stuck", Taakbeheer's restart dialog "{name} closes completely, … and opens again right away" →
  "{name} will close completely, … and open again right away", "Work you have not saved may be lost" → "Any work you
  haven’t saved might be lost" (the wording Start's power dialogs already used).
* **Windows wording**: "(recommended)" → **"(Recommended)"** (Windows' display settings), "{name} properties" →
  **"{name} Properties"** (Alles and Taakbeheer, like Verkenner and the desktop), "Text document" → "Text Document"
  (the desktop's New menu, like Verkenner's), "Windows key + V" → "Windows logo key + V" (like Accounts' "Windows logo
  key + L"), "Unexpected answer" → "Unexpected response" (Taakbeheer, like the bridge), "Under GNOME, GNOME shows
  notifications itself" → "In GNOME, notifications are shown by GNOME itself", "Device name" → "Device Name" (GNOME's
  own label in the System › About path), "Own picture" → "Own photo" (Themes, like Background), "Turn off all wireless
  connections, like …" → "Stop all wireless communication, such as Wi-Fi and Bluetooth".
* **Plural**: Verkenner's, the desktop's and Alles' properties said "1 files, 1 folders" (en dump:
  `explorer.properties.multi`, `desktop.properties`). `containsVal`/`pr_containsVal` are two counts in one text, so a
  plural object cannot carry them; their en value is now a function (`"1 file, 2 folders"`). The nl text is unchanged
  ("1 bestanden, 1 mappen" stays: §8).
* **Width (the pixel rule)**: GE1a's audit found one en-only overflow, the display resolution dropdown ("2560 × 1440
  (recommended)" cut at 220 px). With Windows' capital R it needs about 185 px of text. `display.js` gives its three
  dropdowns `width: lang() === 'en' ? 240 : 220`: Dutch renders exactly as before (220, shot compared), English fits
  at 1280 and 800 px (§7).

Kept on purpose (judged right): "Task view" (Windows 11 Settings' taskbar item; its taskbar tooltip says "Task View",
for Robin), "Quick settings" (sentence case, all 3), "Stored locally" (Windows 11 Home's own tile caption),
"Personalise your device", "Adjust date and time", "Cut and copy history", "Reverting to previous display settings in
…", "Pasting/Extracting …" (progress forms) — each now a glossary context row (§4). "preview/wm.js" en `alles:
'Everything'` → `'Alles'` (cross.md item 8, D-G5: the browser preview WM only).

## 3. The listed UI items

* **cross.md §7 item 6** (`apps/settings/pages/time.js`): the language dropdown's value is `langValue()`, exactly
  contracts C1 (`"en"` → en, anything else → nl), for its first value, after a failed write and on every
  `settings.changed`; a hand-edited "EN" or the object form now shows "Nederlands", the option every layer resolves it
  to. Both dumps are unchanged (the mock's language is a valid value).
* **cross.md §7 item 8**: above.
* **D-G31** (`mock/settings.js`, test `tests/i18n/mock_settings.test.mjs`, 16 assertions): `settings.set` with a
  language string other than "nl"/"en" (also a path below `language`) answers `invalid_value`, data `{path}`, the
  host's text "Deze waarde past niet bij deze instelling." and key `settings.badValue` (settings.rs); a wrong type stays
  `bad_value`; `settings.patch` checks wrong types first (bad_value with the other paths), then the language
  (`invalid_value`, `{paths: ['language']}`), as settings.rs does; `settings.import` drops an invalid language and keeps
  the current one (D-G31's "the current one stays"; before, the mock fell back to the default "nl", and so does H).
  The host's `settings.import` still takes "fr" (settings.rs: sanitize only checks the type): §8 for the coordinator.
* **D-G41** (`mock/index.js`): a thrown mock error keeps `key` and `params` (copied like `core/bridge.js` does), so the
  Update page's immediate refusals carry their catalogue key in mock mode too. The mock's own two texts follow the UI
  language: "Onbekende functie: {method}" / "Unknown function: {method}", "Er ging iets mis." / "Something went wrong."
* **Start tiles at once** (`shell/shell.js`): `onLangChange(() => load())` in the app catalogue, plus a sequence number
  so an `apps.list` still in flight from before the switch cannot overwrite the new list. The 400 ms `apps.changed`
  debounce stays for catalogue changes. Not measured in GNOME (the harness writes into `tests/gnome/`, outside this
  step's files; §8 has the command): the mock's `apps.list` has Dutch names only, so no Chrome dump can show it.
* **The mock's offline text** (`mock/shell.js`): the simulated host outage's `offline` error is `core/bridge.js`'s
  text in the UI language. The en dump's console note now reads "… [offline] No connection to Win11 Shell. Try again
  in a moment." (was Dutch).
* **MOCK-CHANGES.md**: a new section with rows for `mock/update.js` (GW3, GW4, R3-UI), `system.updates.rebootRequired`,
  the `update` settings defaults and D-G31, `mock/shell.js` `shell.restartAll` incl. `pending` (GX1 n9) and the offline
  text, `mock/index.js` (GW3's FILES entry, D-G41, the texts).

## 4. Glossary lock-in

### 4.1 The 52 warnings and what I did

| Warnings | Resolution |
|---|---|
| 15 × Prullenbak → "Trash" or missing (Taakbeheer cleanup.js 5, largefiles.js 9, Alles trashFailed 1) | English fixed: Recycle Bin |
| 7 × Vliegtuigmodus → "Airplane mode" | English fixed: Flight mode |
| 3 × Reageert niet → "isn’t responding" (Bluetooth: bluetooth.js, home.js, quicksettings.js) | English fixed: "is not responding" (D-G34) |
| 3 × Prullenbak leegmaken → "Empty the Recycle Bin?" | English fixed: "Empty Recycle Bin?" |
| 1 × Opslaan → "Saving failed" (Alles) | English fixed: "Couldn’t save" |
| 5 × Terugzetten → Reset (update.js reset/resetBtn/resetDesc/resetQ, the registry's Update en herstel description) | context row → **Reset** (settings back to defaults: Windows' "Reset"; Restore is the Recycle Bin's) |
| 3 × Terugzetten → Revert (display.js keepBody ×2, revert) | context row → **Revert** (Windows' display countdown; the old `glossary-ok` comment had its reason inside the term, so it never applied) |
| 2 × Voorbeeldvenster → "preview window" (about.js, update.js) | context row (Win11 Shell's preview window mode, not the Preview pane) |
| 2 × Plakken → "Pasting", 1 × Alles uitpakken → "Extracting all" (Verkenner job titles) | context rows (progress forms) |
| 1 × Deze pc → "Stored locally" | context row (Windows 11 Home's caption) |
| 1 × Persoonlijke instellingen → "Personalise your device" (Home card) | context row (Windows' own title) |
| 1 × Opslaan → "hold" (battery health) | context row (another sense: a battery holds charge) |
| 1 × Meldingen → "Notification sounds" | context row ("Notification" before a noun) |
| 1 × Afmelden → "Signing out…" | context row (Windows' progress screen) |
| 1 × Afbeeldingen → "images" (search: copied images) | context row (not the Pictures folder) |
| 1 × Datum en tijd → "Adjust date and time" | context row (Windows 11's clock menu item) |
| 1 × Klembordgeschiedenis → "Cut and copy history" (registry) | context row (Windows 11's own Clipboard description) |
| 1 × data.json `timeout` "reageert niet op tijd" → "didn't answer in time" | context row (a timeout, not the status) |
| 1 × data.json `unavailable` Schijfbeheer → "Disk management doesn't work" | context row flagged **known**: an accepted deviation, not a right rendering. The text is pinned by `host/src/services/disks/reference-tests.json` (R2-B); host/src is not R3-UI's. For its owner and Robin (D-G1b, the disks tables) |

The three `// glossary-ok:` comments (update.js `checking`, update-advanced.js `advNotify`, display.js `keepBody`) are
context rows now too; the comment mechanism still works but nothing uses it.

### 4.2 Retry vs Try again (D-G41)

`glossary.tsv` gains **"Opnieuw proberen → Try again"** (the 8 existing buttons outside the Update page say "Try
again"), and one context row gives every `apps/settings/pages/update*.js` file **"Retry"**, as Windows 11's Update page
does (the hero and a failed row already said "Retry"). The glob also covers `update-release.js` (GW5): if GW5's card
says "Opnieuw proberen"/"Try again", check.py fails after the merge (§8).

### 4.3 The mechanism (`tools/i18n/glossary-context.tsv`, check.py)

* Columns: where (comma-separated `<file below the repo>:<key glob>`; the key is a makeT key, `P(<page id>).title|
  description` for a registry page, `pair` for an inline pair, or a host JSON table key), nl phrase, en phrase, flags
  (`-`, `name`, `known`), why. At its places a row replaces the glossary.tsv rendering of the same nl phrase, or adds a
  longer phrase that masks a shorter one ("reageert niet op tijd" over "Reageert niet").
* Rule `glossary-context` (errors): a malformed row, an unknown flag, a place outside `vendor/ui/js/` and `host/src/`
  (the parts check.py visits; check_pairs.py keeps its GLOSSARY_OK), a file glob that matches no checked file, and a
  stale row (no checked text at its places has its nl phrase). A part of the tree the run did not check (`--no-js`) is
  not judged.
* Separate file, not glossary.tsv rows: `check_rust.py` (R3-NATIVE's) reads every non-comment row of glossary.tsv as a
  rule for the whole host; a context row there would become a global rule in check_rust.
* `jsdicts.mjs` now emits `id` on P() pairs (additive; the dictionary snapshot already had it as `pIds`).
* `glossary.py` reads the same rows (through `check.Glossary`): a pair covered by its context row counts as the
  rendering "context", and section 5 lists the rows (known ones marked).

### 4.4 The rule is an error

`check.DEFERRED` and `check_ui.DEFERRED` no longer hold `glossary`; `check_pairs.py` reports its glossary findings as
errors. Tested in `tests/i18n/test_glossary_context.py` (8 tests: the rule's level, the real file loads cleanly with
the one Retry row, replacement only at its places, a longer phrase masks a shorter one, `name` is case-sensitive,
stale/outside/malformed/unknown-flag rows are errors, an unchecked part is not judged, the tree has 0 glossary and 0
context findings).

### 4.5 Not mine

`check_rust.py` has 4 glossary *warnings* of its own (disks.unavailableBody "Schijfbeheer" → "Disk management", the
text the disks oracle pins; sysmon.restart.noEnviron "instellingen" = the environment; update.err.checkFailed
"Controleren op updates" in a sentence; update.part.pixbuf "Afbeeldingen" = images; the same 4 at HEAD): check_rust is
R3-NATIVE's and reads glossary.tsv only, not the context file. They show as `rust` warnings in check.py, so the
lock-in covers the UI, the pairs and the host JSON tables, not yet the host TSV catalogues (§11.4).

## 5. Windows Update phase A2 leftovers (D-G44, D-G45)

* **Held Snap refreshes** (`update-advanced.js`, GW4 review m9): `snaps.held` true → Windows 11 24H2's pause wording:
  `advSnapsHeld` "Updates zijn onderbroken tot {until}. Laatst: {when}." / "Updates are paused until {until}. Last:
  {when}." while the hold has an end (`next`, rendered as an absolute date and time after "tot"/"until": "28-10-2026
  14:35"), `advSnapsHeldForever` "Updates zijn onderbroken. Laatst: {when}." / "Updates are paused. Last: {when}." for
  `next` null. Mock states **S03j** (held 72 hours) and **S03k** (no end) inside mock/update.js' GW4 section; surfaces
  `settings-update-advanced.S03j/S03k` (+ the `_comment` line).
* **The 24H2 parts** §3.6 lacked (GW4 review N8): `advNotifyDesc` under "Melden wanneer opnieuw opstarten vereist is …"
  ("Een melding weergeven wanneer je pc opnieuw moet worden opgestart om het bijwerken te voltooien" / "Show a
  notification when your PC requires a restart to finish updating": Windows' description with the page's "pc"); the
  header `advMore` "Aanvullende opties" / "Additional options" above the rows with icons; on Updategeschiedenis
  "Gerelateerde instellingen" / "Related settings" (common.js's existing `related`) with Windows' Recovery row
  `hRecovery` "Herstel" / "Recovery", `hRecoveryDesc` "Back-up van instellingen, alles terugzetten" / "Settings backup,
  reset everything": it opens Update en herstel with `{section: 'recovery'}` and that page scrolls its backup and
  reset group into view. No "Updates verwijderen" row (there is no uninstall here). A host without `update.history`
  (S02b) shows only its message, as before.
* The 6 new Dutch keys are in `tests/i18n/BASELINE-ADDITIONS.ui` (step `R3-UI`; `test_update_mock_rows.py`'s STEP
  pattern accepts it).
* **The restart-all button** (`update.js`, gx1.md §7 decision 1): after an ok answer of `shell.restartAll` that is not a
  dry run, a page-level `restartAllSent` keeps the InfoBar's "Win11 Shell opnieuw starten" disabled through every
  repaint until the page is built again (`write()` got an `onOk` hook that runs before its repaint). A dry run or a
  refusal leaves it usable. Not on a dump surface (the dump shows no disabled state).
* **GX1 n9**: the MOCK-CHANGES.md row for `mock/shell.js` `shell.restartAll` (incl. `pending`).

## 6. Baselines (re-recorded only through run.sh --record --accept)

**nl, 16 changed blocks, all D-G44** (BASELINE-CHANGES.ui line `R3-UI`): `settings.update-history`,
`settings.update-advanced`, `settings-update-history.S01`, `.S01b`, `.S01c`, `.S01d`, `.S02`,
`settings-update-advanced.S03`, `.S03b` … `.S03i`. The only added lines: "Een melding weergeven wanneer je pc opnieuw
moet worden opgestart om het bijwerken te voltooien", "Aanvullende opties", "Gerelateerde instellingen", "Herstel",
"Back-up van instellingen, alles terugzetten"; no line removed or changed (checked over all 16 blocks). New blocks:
`settings-update-advanced.S03j`, `.S03k`. Every other nl block is byte-identical.

**en, 43 changed blocks** (all English, reasons in §2/§5; exact hunks in `.rt/r3-ui/en-1.txt` against
`.rt/r3-ui/baseline-en.before.txt`):

| Block(s) | Why |
|---|---|
| `shell.quick`, `settings.network` | Flight mode; "Stop all wireless communication, such as Wi-Fi and Bluetooth" |
| `shell.power.reboot`, `settings-update.H10d` | "the PC will restart by itself" |
| `shell.power.logout` | "you’ll return to the sign-in screen" |
| `desktop.properties`, `explorer.properties.multi` | "4 files, 1 folder", "1 file, 1 folder" |
| `desktop.emptytrash` | "Empty Recycle Bin?" |
| `settings.display` | "(Recommended)" |
| `settings.power` | "When you’re not using the computer" (2) |
| `settings.privacy` | "Deleted files go to the Recycle Bin" |
| the 16 D-G44 blocks | their en lines ("Show a notification …", "Additional options", "Related settings", "Recovery", "Settings backup, reset everything") |
| `taskmgr.summary` | "Here’s what’s going on." |
| `taskmgr.services` | "aren’t listed, so you can’t stop …", "can’t be stopped" (tooltips) |
| `taskmgr-dialog.end`, `taskmgr-dialog.restart` | "Any work you haven’t saved might be lost.", "that’s safer", "Firefox will close completely, … and open again right away." |
| `taskmgr-dialog.properties` | "Firefox Properties" |
| `taskmgr-dialog.restart-shell` | "Task Manager will reconnect automatically." |
| `explorer.share` | "isn’t available" |
| `alles.options`, `alles.options-bottom` | "Builds the overview again from scratch." |
| `alles.properties` | "… Properties" |
| `alles.error`, `alles.empty` | "This search isn’t quite right", "Hidden files aren’t shown." |
| `disks.format`, `disks.delete`, `disks.new`, `disks.initialize` | "Without the password, nobody … the files are lost.", "This makes sure you’ve chosen the right volume/disk.", "That data will be erased too." |

New en blocks: S03j ("Updates are paused until 28/10/2026 14:35. Last: 7 hours ago (25/10/2026 07:35).") and S03k.

## 7. Screenshots (shoot.sh, Chrome in nonet.sh's layer; my review against Windows 11 24H2 from memory)

nl + en at 1280×1400: `settings.display`, `settings.network`, `shell.quick`, `settings.update-advanced`,
`settings-update-advanced.S03j`, `.S03k`, `settings.update-history`, `desktop.emptytrash`, `desktop.properties`,
`taskmgr-dialog.end`, `taskmgr-dialog.restart`, `explorer.properties.multi`, `settings.language`; en at 800×1400:
`settings-update-advanced.S03j`, `settings.update-history`, `settings.display`.

* Display: en "2560 × 1440 (Recommended)" and "100% (Recommended)" fit the 240 px dropdowns at 1280 and 800 px; nl
  renders at 220 px exactly as before ("(aanbevolen)").
* Geavanceerde opties: the description sits under the notification row like Windows' (one line at 1280, two at 800);
  "Aanvullende opties" / "Additional options" is a group header like Windows' above the three icon rows; the held text
  is one line at 1280 px in both languages and wraps at "(25/10/2026 07:35)" at 800 px. No clipping.
* Updategeschiedenis: "Related settings" header and the Recovery card with chevron under the groups, as in Windows.
* Quick settings: "Flight mode" fits its tile label (shorter than "Vliegtuigmodus"); Taakbeheer's end/restart dialogs,
  the Empty Recycle Bin dialog and the properties dialogs: the longer en sentences wrap inside the dialog, nothing cut.
* Nothing overflows in any changed surface; no Dutch UI text in en.

## 8. Left, for Robin, for other steps, decisions for the coordinator

**For Robin (D-G1b, not changed):** "Nederlands (Nederland)", "Afbeelding", "{n} items konden …" (unchanged from round
1); the Dutch "1 bestanden, 1 mappen" in the properties dialogs (the en side is fixed; the nl singular is a Dutch
change no decision covers); "Task view" (Settings' wording) vs the taskbar tooltip's "Task View"; the host's "Disk
management doesn't work: …" (the `known` context row; pinned by the disks oracle) and the other disks-table
apostrophes; "(ApiError)" in `storage.trashFailed` and the English regex detail in the Dutch Alles message (plan.md
round-2 list).

**For other steps / the coordinator:**
1. `apps/taskmgr/pages/settings.js` (R3-NATIVE's file) needs the same English pass: `confirmDesc` "You get an
   explanation first, so you do not lose work." → "You’ll get an explanation first, so you don’t lose any work.",
   `saveFailed` "Could not save the setting" → "Couldn’t save the setting", `testMode` "Test mode: actions are not
   really carried out" → "Test mode: actions aren’t really carried out" (visible on `taskmgr.settings`).
2. `mock/update.js`'s file header (outside its GW4 section, so not edited) still says "a thrown error … carries only
   code + message, because mock/index.js's invoke rebuilds it from those two": stale since D-G41's fix; one line for
   whoever owns the header (GW5 is in that file now).
3. GW5 merge: with the glossary rule an error, any new GW5 text that misses a glossary phrase fails check.py; the
   Update-page Retry row also covers `update-release.js`, so its retry button must say "Retry" (or the row's glob is
   narrowed to `update.js`).
4. D-G31's host side: `host/src/settings.rs` `import()` still takes a language string other than nl/en (sanitize checks
   the type only); the mock now drops it and keeps the current one. R3-NATIVE or a later step.
5. A GNOME-harness check of the Start tiles after a switch (the harness writes into `tests/gnome/`, not R3-UI's):
   `tools/heavy.sh --min-mb 2560 --tag r3-start -- python3 -B tests/gnome/harness.py launch run --lang nl --build-dir
   tests/build/base-debug --source worktree --scenario tests/gnome/scenarios/ge1a-switch.py --out
   tests/gnome/scenarios/ge1a-switch.r3ui` (step 7's `cleanFromMs` should drop from the earlier 326-560 ms).
6. `check_rust.py` could read `glossary-context.tsv` for host rows (its 4 glossary warnings, §4.5).
7. `notifcenter.js` `unsupported` still names "de Win11 Shell-sessie" (D-G2 class); per GE1a it never shows in GNOME,
   so I left it (a Dutch change would need a D-G2 line).
8. A transient case GE1a listed is still open: a dialog open during a language switch keeps its default buttons; an
   open search list keeps "Geen resultaten" until the next keystroke.
9. Both empty-name aria-labels "Submappen van " / "Subfolders of " (29 × on `explorer.home`, both languages): the
   breadcrumb root's chevron has no name; a UI bug, not an English one.

**Deviations:** (a) the context rows are a new file, not rows of glossary.tsv (§4.3); (b) `display.js` sets a
per-language width (the smallest fix that keeps Dutch identical, ge1a decision 4); (c) en `containsVal` is a function
value (the first in a makeT dictionary; jsdicts reads it as `fn`); (d) D-G31's import rule follows the decision's text
("the current one stays"), which differs from H and the old mock (the default); (e) no GNOME-harness run (above).

## 9. Files changed

* `vendor/ui/js/` (49 modules): the English review in `apps/alles/i18n.js`, `apps/disks/i18n.js`,
  `apps/explorer/{i18n,picker}.js`, `apps/installer/main.js`, `apps/settings/main.js`, `apps/settings/registry.js`,
  `apps/settings/pages/{bluetooth,display,home,network,notifications,power,privacy,system,themes,update}.js`,
  `apps/taskmgr/{actions,hostbanner,hostcall,main}.js`, `apps/taskmgr/pages/{services,startup,summary}.js`,
  `apps/taskmgr/storage/{categories,cleanup,explorer,largefiles}.js`, `core/{bridge,software}.js`,
  `desktop/strings.js`, `main.js`, `preview/wm.js`, `shell/{clipboard,notifcenter,power,quicksettings,search,shell,
  startmenu,taskbar,tray}.js`; behaviour in `apps/settings/pages/{display,time,update,update-advanced,
  update-history}.js`, `shell/shell.js`, `mock/{index,settings,shell}.js`, `mock/update.js` (GW4 section only).
* `vendor/ui-sha256.json`, `vendor/ui-patches.json` (regenerated; entry `round3_english_review`).
* `tests/i18n/`: `baseline-{nl,en}.txt` (recorded), `surfaces.json` (+2), `BASELINE-CHANGES.ui` (+1 line),
  `BASELINE-ADDITIONS.ui` (+6 lines), `MOCK-CHANGES.md` (new section), `test_update_mock_rows.py` (STEP), new
  `test_glossary_context.py`, new `mock_settings.test.mjs`.
* `tools/i18n/`: `check.py` (context rows, rule `glossary-context`, glossary an error), `check_ui.py` (where, DEFERRED),
  `check_pairs.py` (glossary errors), `glossary.py` (context-aware report), `glossary.tsv` (header, "Opnieuw proberen →
  Try again"), new `glossary-context.tsv` (18 rows), `jsdicts.mjs` (P() pair `id`).
* This note.

## 10. Every English change of the review (before → after; `.rt/r3-ui/en_fix.log`)

Besides these 175: `containsVal` (Verkenner, desktop) and `pr_containsVal` (Alles) "{files} files, {folders} folders"
→ the plural-aware function; `preview/wm.js` `alles` "Everything" → "Alles"; the new en texts of §5.

**`apps/alles/i18n.js`**

| Before | After |
|---|---|
| Alles is making an overview of your files | Alles is building an overview of your files |
| After that you will find every file and folder in an instant, as you type. | After that, you’ll find every file and folder instantly as you type. |
| You can close this window. The overview is finished in the background. | You can close this window. The overview will be finished in the background. |
| Nothing of kind “{f}” matches “{q}”. | Nothing of the kind “{f}” matches “{q}”. |
| Hidden files are not shown. You can turn this on in {link}. | Hidden files aren’t shown. You can turn this on in {link}. |
| This search is not quite right | This search isn’t quite right |
| The overview of your files could not be reached. Please try again in a moment. | The overview of your files couldn’t be reached. Try again in a moment. |
| “{name}” goes to the Recycle Bin. You can restore it from there later. | “{name}” will be moved to the Recycle Bin. You can restore it from there later. |
| The {n} selected items go to the Recycle Bin. You can restore them from there later. | The {n} selected items will be moved to the Recycle Bin. You can restore them from there later. |
| Everything inside this folder goes too. | Everything in this folder will be moved too. |
| Not everything could be moved | Not everything could be moved to the Recycle Bin |
| item could not be moved. You may not have permission. | item couldn’t be moved. You might not have permission. |
| items could not be moved. You may not have permission. | items couldn’t be moved. You might not have permission. |
| Copying failed | Couldn’t copy |
| Could not open this | Couldn’t open this |
| {name} properties | {name} Properties |
| Makes the overview from scratch. This takes a few minutes; searching keeps working. | Builds the overview again from scratch. This takes a few minutes; searching keeps working. |
| Alles reads all your drives again. This can take a few minutes and your computer may be a bit slower meanwhile. You can keep searching. | Alles will read all your drives again. This can take a few minutes, and your computer may be a bit slower in the meantime. You can keep searching. |
| Saving failed | Couldn’t save |

**`apps/disks/i18n.js`**

| Before | After |
|---|---|
| Windows manages the volumes on this disk itself. Better change them in Windows; they can only be deleted there. | Windows manages the volumes on this disk itself. It’s best to change them in Windows; they can only be deleted there. |
| {names} starts from this volume. That’s why it can’t be formatted, deleted or renamed, otherwise {names} won’t start any more. | {names} starts from this volume. That’s why it can’t be formatted, deleted or renamed: {names} wouldn’t start any more. |
| {names} starts from a volume on this disk. That’s why it can’t be initialised. Safely removing it is fine: {names} starts again once the disk is back. | {names} starts from a volume on this disk. That’s why it can’t be initialised. You can still safely remove it: {names} will start again once the disk is back. |
| Without the password nobody can read the files. If you forget it, they are gone. | Without the password, nobody can read the files. If you forget it, the files are lost. |
| So you’re sure you chose the right volume. | This makes sure you’ve chosen the right volume. |
| So you’re sure you chose the right disk. | This makes sure you’ve chosen the right disk. |
| RAID system). That is erased too. | RAID system). That data will be erased too. |

**`apps/explorer/i18n.js`**

| Before | After |
|---|---|
| This tab could not be moved because its data is not valid. | This tab couldn’t be moved because its data isn’t valid. |
| This tab could not be opened in this window. | This tab couldn’t be opened in this window. |
| Recent files are not being tracked. You can turn this on in Options. | Recent files aren’t being tracked. You can turn this on in Options. |
| Put photos in your Pictures folder and they will appear in the Gallery automatically. | Put photos in your Pictures folder and they’ll appear in the Gallery automatically. |
| The Recycle Bin was not found on this computer. | The Recycle Bin couldn’t be found on this computer. |
| This went wrong for: {names} | There was a problem with: {names} |
| Empty the Recycle Bin? | Empty Recycle Bin? |
| Who may change this is set in the permissions. | The permissions decide who can change this. |
| Sharing directly is not available in File Explorer yet.  | Sharing directly isn’t available in File Explorer yet.  |

**`apps/explorer/picker.js`**

| Before | After |
|---|---|
| This file does not exist. | This file doesn’t exist. |

**`apps/installer/main.js`**

| Before | After |
|---|---|
| This action is not available. | This action isn’t available. |

**`apps/settings/main.js`**

| Before | After |
|---|---|
| Something went wrong. Please try again. | Something went wrong. Try again. |

**`apps/settings/pages/bluetooth.js`**

| Before | After |
|---|---|
| Bluetooth isn’t responding | Bluetooth is not responding |
| Bluetooth works again | Bluetooth is working again |
| Try once more. If that doesn’t help, restart the computer. | Try again. If that doesn’t help, restart the computer. |
| Searching for devices failed. | Couldn’t search for devices. |
| Don’t see your device? Put it in pairing mode: usually you hold the power or Bluetooth button for a few seconds until a light blinks. | Don’t see your device? Put it in pairing mode: usually by holding its power or Bluetooth button for a few seconds until a light flashes. |

**`apps/settings/pages/home.js`**

| Before | After |
|---|---|
| Bluetooth isn’t responding | Bluetooth is not responding |

**`apps/settings/pages/display.js`**

| Before | After |
|---|---|
| {v} (recommended) | {v} (Recommended) |

**`apps/settings/pages/network.js`**

| Before | After |
|---|---|
| Airplane mode | Flight mode |
| Turn off all wireless connections, like Wi-Fi and Bluetooth | Stop all wireless communication, such as Wi-Fi and Bluetooth |
| Network information isn’t available right now. Please try again later. | Network information isn’t available right now. Try again later. |
| Other people on this network might see what you do. Better not use it for banking. | Other people on this network might see what you do. It’s best not to use it for banking. |
| You will no longer be connected to the internet through this network. | You’ll no longer be connected to the internet through this network. |

**`apps/settings/pages/notifications.js`**

| Before | After |
|---|---|
| Under GNOME, GNOME shows notifications itself. These settings apply to Win11 Shell. | In GNOME, notifications are shown by GNOME itself. These settings apply to Win11 Shell. |

**`apps/settings/pages/power.js`**

| Before | After |
|---|---|
| The power mode cannot be changed on this computer. | The power mode can’t be changed on this computer. |
| The computer is too warm. | The computer is too hot. |
| When you are not using the computer | When you’re not using the computer |
| Sleep automatically when you are not using the computer | Sleep automatically when you’re not using the computer |
| This setting is not available on this computer. | This setting isn’t available on this computer. |
| This computer does not currently support automatic sleep. | This computer doesn’t support automatic sleep right now. |
| This setting is managed and cannot be changed here. | This setting is managed and can’t be changed here. |
| The power settings could not be loaded. | The power settings couldn’t be loaded. |

**`apps/settings/pages/privacy.js`**

| Before | After |
|---|---|
| The list of recently opened files will be emptied, also in other apps that use this list. Your files themselves are not deleted. | The list of recently opened files will be emptied, including in other apps that use this list. Your files themselves aren’t deleted. |
| Deleting goes to the Recycle Bin | Deleted files go to the Recycle Bin |

**`apps/settings/pages/system.js`**

| Before | After |
|---|---|
| This PC is called “{name}”. You change the name in Ubuntu: open Ubuntu’s Settings app and go to System › About › Device name. Ubuntu will ask for your password. | This PC is called “{name}”. To change the name, open Ubuntu’s Settings app and go to System › About › Device Name. Ubuntu will ask for your password. |

**`apps/settings/pages/themes.js`**

| Before | After |
|---|---|
| Own picture | Own photo |

**`apps/settings/pages/update.js`**

| Before | After |
|---|---|
| All apps will close and the PC starts again by itself. Any work you haven’t saved might be lost. | All apps will close and the PC will restart by itself. Any work you haven’t saved might be lost. |

**`apps/settings/registry.js`**

| Before | After |
|---|---|
| Wi-Fi, Ethernet, airplane mode | Wi-Fi, Ethernet, flight mode |
| ['network', 'internet', 'connection', 'online', 'airplane mode', 'offline'] | ['network', 'internet', 'connection', 'online', 'flight mode', 'airplane mode', 'offline'] |

**`apps/taskmgr/actions.js`**

| Before | After |
|---|---|
| Work you have not saved may be lost. | Any work you haven’t saved might be lost. |
| Tip: if you can still close the app yourself, that is safer. | Tip: if you can still close the app yourself, that’s safer. |
| {name} could not be ended | {name} couldn’t be ended |
| System process – cannot be ended | System process – can’t be ended |
| Efficiency mode could not be changed | Efficiency mode couldn’t be changed |
| {name} properties | {name} Properties |
| Something went wrong. Please try again. | Something went wrong. Try again. |
|  That often helps when something got stuck. |  This often helps when something is stuck. |
| Work that has not been saved may be lost. | Any work you haven’t saved might be lost. |
| ‘{proc}’ belongs to {name}. So {name} closes completely, with all its windows, and opens again right away. | ‘{proc}’ belongs to {name}, so {name} will close completely, with all its windows, and open again right away. |
| {name} closes completely, with all its windows, and opens again right away. | {name} will close completely, with all its windows, and open again right away. |
| {name} could not be restarted | {name} couldn’t be restarted |
| System process – cannot be restarted | System process – can’t be restarted |
| This version of Win11 Shell cannot restart processes yet. | This version of Win11 Shell can’t restart processes yet. |
| Win11 Shell could not be restarted | Win11 Shell couldn’t be restarted |
| Test mode: Win11 Shell was not actually restarted | Test mode: Win11 Shell wasn’t actually restarted |
| The taskbar and the desktop start again. | The taskbar and the desktop will restart. |

**`apps/taskmgr/hostbanner.js`**

| Before | After |
|---|---|
| Task Manager keeps trying. What you see below is from {time}. | Task Manager will keep trying. What you see below is from {time}. |
| Task Manager keeps trying and reconnects by itself. | Task Manager will keep trying and reconnect automatically. |
| Win11 Shell is still not back. Please try once more. | Win11 Shell still isn’t back. Try again. |
| Please wait. Task Manager reconnects by itself. | Please wait. Task Manager will reconnect automatically. |

**`apps/taskmgr/hostcall.js`**

| Before | After |
|---|---|
| Win11 Shell is not responding. Please try again in a moment. | Win11 Shell is not responding. Try again in a moment. |
| No connection to Win11 Shell. Please try again in a moment. | No connection to Win11 Shell. Try again in a moment. |
| No access: this page does not have a valid key. | No access: this page doesn’t have a valid key. |
| Unexpected answer from Win11 Shell. | Unexpected response from Win11 Shell. |

**`apps/taskmgr/main.js`**

| Before | After |
|---|---|
| This page could not be loaded | This page couldn’t be loaded |
| Please try again. If that does not help, close Task Manager and open it again. | Try again. If that doesn’t help, close Task Manager and open it again. |
| Search could not be opened | Couldn’t open search |

**`apps/taskmgr/pages/services.js`**

| Before | After |
|---|---|
| System services are not listed, so you cannot stop anything important by accident. | System services aren’t listed, so you can’t stop anything important by accident. |
| Needed by your desktop – cannot be stopped | Needed by your desktop – can’t be stopped |
| The service stops briefly and starts again. That often helps when something got stuck. | The service stops briefly and starts again. This often helps when something is stuck. |
| That did not work | That didn’t work |

**`apps/taskmgr/pages/startup.js`**

| Before | After |
|---|---|
| Could not change this | Couldn’t change this |

**`apps/taskmgr/pages/summary.js`**

| Before | After |
|---|---|
| Here is what is going on. Fix it with one click. | Here’s what’s going on. Fix it with one click. |
| Could not open this | Couldn’t open this |

**`apps/taskmgr/storage/categories.js`**

| Before | After |
|---|---|
| Could not calculate the categories | Couldn’t calculate the categories |
| Click a category to see what is in it. | Click a category to see what’s in it. |

**`apps/taskmgr/storage/cleanup.js`**

| Before | After |
|---|---|
| Free up space by cleaning files your PC no longer needs. You see exactly what goes. | Free up space by cleaning up files your PC no longer needs. You’ll see exactly what will be removed. |
| A few items live in system folders. Ubuntu first asks for your password – that is normal and safe. | A few items are in system folders, so Ubuntu asks for your password first – that’s normal and safe. |
| Goes to the Trash | Goes to the Recycle Bin |
| Note: what you clean from the Trash cannot be restored. | Note: what you clean up from the Recycle Bin can’t be restored. |
| The chosen downloads go to the Trash; you can still restore them from there. | The chosen downloads will be moved to the Recycle Bin; you can still restore them from there. |
| 1 download is now in the Trash ({size}) | 1 download is now in the Recycle Bin ({size}) |
| {n} downloads are now in the Trash ({size}) | {n} downloads are now in the Recycle Bin ({size}) |
| Skipped because they are still open: {names}. Close them and try again. | Skipped because they’re still open: {names}. Close them and try again. |
| Logs total {total}; everything older than {days} days goes. | Logs take up {total}; everything older than {days} days will be removed. |
| What is in here? | What’s in here? |
| Could not load the list | Couldn’t load the list |

**`apps/taskmgr/storage/explorer.js`**

| Before | After |
|---|---|
| Choosing is not available here | Choosing isn’t available here |
| Win11 Shell is not responding. Scan again once it works again. | Win11 Shell is not responding. Scan again once it’s working again. |

**`apps/taskmgr/storage/largefiles.js`**

| Before | After |
|---|---|
| Move to Trash | Move to Recycle Bin |
| Files go to the Trash. You can restore them from there until you empty the Trash. | Files are moved to the Recycle Bin. You can restore them from there until you empty the Recycle Bin. |
| Move 1 file to the Trash? | Move 1 file to the Recycle Bin? |
| Move {n} files to the Trash? | Move {n} files to the Recycle Bin? |
| This frees {size} once you empty the Trash. Until then you can restore everything. | This frees up {size} once you empty the Recycle Bin. Until then, you can restore everything. |
| 1 file is in the Trash | 1 file is in the Recycle Bin |
| {n} files are in the Trash | {n} files are in the Recycle Bin |
| You can restore them from the Trash | You can restore them from the Recycle Bin |
| Open Trash | Open Recycle Bin |
| 1 file could not be moved | 1 file couldn’t be moved |
| {n} files could not be moved | {n} files couldn’t be moved |
| Moving did not work | Couldn’t move the files |
| Could not load the list | Couldn’t load the list |
| Leave it unless you are sure you no longer need it. | Leave it unless you’re sure you no longer need it. |

**`core/bridge.js`**

| Before | After |
|---|---|
| No connection to Win11 Shell. Please try again in a moment. | No connection to Win11 Shell. Try again in a moment. |
| No access: this page does not have a valid key. | No access: this page doesn’t have a valid key. |
| The test environment could not be loaded. | The test environment couldn’t be loaded. |

**`core/software.js`**

| Before | After |
|---|---|
| This action is not available. | This action isn’t available. |
| Cancellation isn’t available right now | Can’t cancel right now |
| Check Activity before trying again. Review the operation again before a new attempt. | Check Activity before you try again. To try again, review the operation once more. |

**`desktop/strings.js`**

| Before | After |
|---|---|
| Text document | Text Document |
| Win11 Shell runs in test mode: nothing was moved to the Recycle Bin. | Win11 Shell is running in test mode: nothing was moved to the Recycle Bin. |
| Win11 Shell runs in test mode: this change was not really made. | Win11 Shell is running in test mode: this change wasn’t actually made. |
| Empty the Recycle Bin? | Empty Recycle Bin? |
| The name could not be changed. | The name couldn’t be changed. |
| This item could not be opened. | This item couldn’t be opened. |
| Paste didn’t work. | Couldn’t paste. |
| Something went wrong. Please try again. | Something went wrong. Try again. |
| Please enter a name. | Enter a name. |
| The attributes (Read-only, Hidden) could not be changed. | The attributes (Read-only, Hidden) couldn’t be changed. |

**`main.js`**

| Before | After |
|---|---|
| This part could not be loaded | This part couldn’t be loaded |
| Something went wrong while starting {what}. Please try again; if that does not help, restart Win11 Shell. | Something went wrong while starting {what}. Try again; if that doesn’t help, restart Win11 Shell. |
| This app does not exist | This app doesn’t exist |
| The app “{app}” was not found. | The app “{app}” couldn’t be found. |

**`preview/wm.js`**

| Before | After |
|---|---|
| alles: 'Everything' | alles: 'Alles' |

**`shell/clipboard.js`**

| Before | After |
|---|---|
| Copy some text or an image and it will show up here. Press Windows key + V to open this panel. | Copy some text or an image and it’ll show up here. Press Windows logo key + V to open this panel. |
| That didn’t work. Please try again. | That didn’t work. Try again. |

**`shell/notifcenter.js`**

| Before | After |
|---|---|
| That didn’t work. Please try again. | That didn’t work. Try again. |

**`shell/power.js`**

| Before | After |
|---|---|
| All apps will close and the PC starts again by itself. Any work you haven’t saved might be lost. | All apps will close and the PC will restart by itself. Any work you haven’t saved might be lost. |
| All apps will close and you return to the sign-in screen. Any work you haven’t saved might be lost. | All apps will close and you’ll return to the sign-in screen. Any work you haven’t saved might be lost. |
| The PC couldn’t do this right now. Please try again. | The PC couldn’t do this right now. Try again. |

**`shell/quicksettings.js`**

| Before | After |
|---|---|
| Airplane mode | Flight mode |
| Airplane mode is on | Flight mode is on |
| Turn off airplane mode to connect again. | Turn off flight mode to connect again. |
| Bluetooth isn’t responding. | Bluetooth is not responding. |
| Bluetooth works again. | Bluetooth is working again. |
| That didn’t work. Please try again. | That didn’t work. Try again. |

**`shell/search.js`**

| Before | After |
|---|---|
| Everything in the Recycle Bin will be deleted for good. This cannot be undone. | Everything in the Recycle Bin will be deleted for good. This can’t be undone. |
| That did not work | That didn’t work |
| Empty the Recycle Bin? | Empty Recycle Bin? |
| wifi wi-fi ethernet flight airplane internet | wifi wi-fi ethernet flight mode airplane internet |

**`shell/shell.js`**

| Before | After |
|---|---|
| {name} could not be started | {name} couldn’t be started |
| This change could not be saved | This change couldn’t be saved |
| This could not be opened | This couldn’t be opened |
| Your screen will be locked. You need your password to continue. | Your screen will be locked. You’ll need your password to continue. |
| That did not work | That didn’t work |

**`shell/startmenu.js`**

| Before | After |
|---|---|
| The more you use your computer, the more recent files and new apps you will see here. | The more you use your computer, the more recent files and new apps you’ll see here. |

**`shell/taskbar.js`**

| Before | After |
|---|---|
| Airplane mode is on | Flight mode is on |

**`shell/tray.js`**

| Before | After |
|---|---|
| The order of the icons could not be saved | The order of the icons couldn’t be saved |

## 11. Fix round (after `r3-ui-review.md`, 2026-11-06)

**Start state.** W matched the review: HEAD 18c8ffbe, 64 modified and 4 new paths (the note directory, two tests,
`glossary-context.tsv`), `check.py` 0 errors / 72 `rust` warnings, baselines 24786 / 24783 lines. Nothing committed.
Scratch of this round: `.rt/r3-ui/fix/` (the replacement script `fix_en.py` and its log, dumps, accept list, shots).

### 11.1 Findings

| Id | Result | What |
|---|---|---|
| m1 | fixed | `taskmgr/pages/details.js`, `processes.js` `systemProc` → "System process – can’t be ended" (one rendering with `actions.js`) |
| m2 | fixed | Alles `noResultsFilter` → "No items of kind “{f}” match “{q}”." ("kind" as Alles' own filter label "Kind" and hint3 say; not "type", which is the column) |
| m3 | fixed | `endBodyMany` → "All {n} processes of {name} will close. …" (one: "{n} process of {name} will close. …"); `retryReview` → "Check Activity and review the operation once more before you try again."; storage `hostDown` → "… Scan again once it’s back." (like hostbanner's "still isn’t back"); `opt_rebuildDesc` → "Rebuilds the overview from scratch. …"; `queryError` → "There’s a problem with this search". Found by the same scan: Bluetooth `removeBody` "To use it again later, … add it again" → "To use it later, …" |
| m4 | fixed | Alles' type names in Windows' Title Case like Verkenner's `TYPES_EN`/`extFile`: "{ext} File", "{ext} Image/Audio/Video/Archive", "Text Document", "PDF/Word/Markdown Document", "Excel Worksheet", "PowerPoint Presentation", "OpenDocument Text/Spreadsheet/Presentation", "Disc Image", "Installer Package", "Virtual Disk", "Web Page", "Source Code ({ext})" ("File folder" stays, as Windows writes it); Verkenner's four sentence-case leftovers in `explorer/util.js` `TYPES_EN` → "Software Package", "AppImage Program", "Windows Program", "Shell Script". The Empty Recycle Bin message ("… definitief verwijderd. Dit kun je niet ongedaan maken.") has one rendering in Verkenner, search and the desktop: "… will be permanently deleted. This can’t be undone." ("permanently" as the desktop's button "Delete permanently" and `trashEmptiedMsg`; "This can’t be undone." as clipboard and update reset) |
| m5 | fixed (note) / coordinator | §1, §4.5 and §8 item 6 say 4 (incl. `disks.unavailableBody`). The host TSV lock-in is R3-NATIVE's `check_rust.py` (§11.4) |
| m6 | partly fixed | New dump surfaces with a `probe` (below) cover the restart-all button, the Recovery deep link and the hand-edited language; `mock_index.test.mjs` covers the router. **Not done:** the Start tiles' immediate reload, which only the GNOME harness can show (§11.4) |
| n1 | fixed | `update.js`: `section: 'recovery'` is a one-shot param (`delete ctx.params.section`, like `bluetooth.js`' `add`), so a keepScroll re-render (language switch, refresh) keeps the user's position |
| n2 | fixed | `mock/index.js` `mkErr` copies C2's `data` too; MOCK-CHANGES.md row updated |
| n3 | not changed | "your PC" / "je pc" kept for the page's consistency; for Robin (24H2 says "your device" / "je apparaat") |
| n4 | not changed | for the coordinator (§11.4) |
| n5 | not changed | the `known` row stays until the disks oracle text changes (its owner or Robin) |
| n6 | handed on | cross.md §7 item 9 (the extension's "Win11 taskbar" vs "Win11 Taskbar") is `extension/`, not R3-UI's: §11.4 |
| n7 | not changed | outside this step's files/sections (mock/update.js header: GW5's part of the file now; notifcenter.js `unsupported` needs a D-G2 line) |
| n8 | kept, with reason | the Retry row's glob stays `update*.js`: `update-release.js` renders `upd.release` on the Update page itself (between the InfoBars and the rows), and Windows 11's Update page says "Retry" there too; narrowing it would let GW5's card say "Try again" next to the hero's "Retry". The coordinator runs check.py right after the GW5 merge |

### 11.2 Tests for m6

* `tests/i18n/dump.html`: an entry's optional `probe` (`[{disabled|inview: selector, nth?}]`) runs after the
  actions and adds one line per check, `# probe disabled|inview <selector>: true|false|none`. Only entries with a
  probe get these lines: every other block is byte-identical (both full dumps equal the old blocks).
* `vendor/ui/js/mock/shell.js`: `?mockRestartAll=live` makes `shell.restartAll` answer `dryRun: false` (also its
  `pending` answer); without it nothing changes. Classes for stable selectors: `st-upd__backup` (the backup and reset
  group, `update.js`), `st-updhist__recovery` (the Herstel card, `update-history.js`); no visible change.
* Five surfaces `settings-check.*` (new blocks in both baselines):
  * `language-EN`: seed `{"language": "EN"}`: the dropdown shows "Nederlands", C1's reading (was the placeholder).
  * `restartall-dry`: H20, click "Win11 Shell opnieuw starten": `disabled … : false` (a dry run leaves it usable).
  * `restartall-sent`: the same with `mockRestartAll=live`: `disabled … : true` (gx1.md §7 decision 1).
  * `recovery-link`: Updategeschiedenis (S01, 1280×520), click Herstel: Update en herstel opens with
    `.st-upd__backup` in view (`true`); `recovery-none` is the control (the page itself at the same size: `false`).
* `tests/i18n/test_ge0a.py`: `test_surfaces` validates the keys of a surface and of a probe; new
  `test_probes_hold` asserts the five expected lines in both baselines.
* New `tests/i18n/mock_index.test.mjs` (7 assertions, no socket call under strace): through `createMock`, D-G31's
  refusals keep code, key, params and data; an error without key stays without; the router's own text in en; the
  default and the `live` restartAll answers.
* Not covered: n1 (a re-render after a language switch on the Update page; the dump cannot switch the language on a
  page) and the Start tiles (§11.4).

### 11.3 Baselines and verification (commands from W's root, `PATH=/home/robin/.local/node/bin:$PATH`)

Re-recorded with `tools/heavy.sh --tag r3-ui-dump -- tests/i18n/run.sh --record` (nl: no changed block, 5 new) and
`… --record --lang en --accept desktop.emptytrash,taskmgr-dialog.end,alles.options,alles.options-bottom,alles.preview,alles.properties,alles.error`
(en: those 7 changed blocks, 5 new). Both recorded files equal the scratch dumps made before (`cmp`).

| en block | Before → after |
|---|---|
| `desktop.emptytrash` | "… will be permanently deleted. You can’t undo this." → "… This can’t be undone." |
| `taskmgr-dialog.end` | "This closes 17 processes of Firefox." → "All 17 processes of Firefox will close." |
| `alles.options`, `alles.options-bottom` | "Builds the overview again from scratch." → "Rebuilds the overview from scratch." |
| `alles.preview`, `alles.properties` | "XCF image" → "XCF Image" |
| `alles.error` | "This search isn’t quite right" → "There’s a problem with this search" |

**Dutch:** no nl block changed, no nl dictionary value changed (dict diff unchanged: 2925 → 3170 rows, 0 unlisted,
0 stale). `BASELINE-CHANGES.ui` got one "Not a change: new blocks" line (as GE1a's) for the five `settings-check.*`
blocks; `BASELINE-ADDITIONS.ui` is unchanged (no new Dutch key).

| Check | Result |
|---|---|
| `python3 -B tools/i18n/check.py` | 0 errors, 72 warnings (all `rust`, of which 4 `[glossary]` from check_rust, §4.5); 0 under rule `glossary`; 18 context rows, none stale |
| `python3 -B tools/i18n/ratchet.py` | `ui: ok (89 allowed occurrences, 0 still to convert)` |
| `python3 -B tools/i18n/check_ui.py --dict-diff tests/i18n/dict-nl-3b898e6.tsv --quiet` | 2925 → 3170 rows; 9 changed, 20 removed, 265 added; 0 not in BASELINE-CHANGES.ui, 0 stale |
| `tools/heavy.sh --tag r3-ui-dump -- tests/i18n/run.sh` / `--lang en` | `nl dump equals baseline-nl.txt (25125 lines)` / `en dump equals baseline-en.txt (25122 lines)` |
| `… run.sh --twice` / `--twice --lang en` | `two nl dumps identical (25125 lines, sha256 6c90963063b3f012)` / `two en dumps identical (25122 lines, sha256 f180e1a39c00fe0c)` |
| `node --input-type=module --check` | the 52 changed modules, `jsdicts.mjs` and the three node tests: 56 files, 0 failed |
| node tests | `core_i18n.test.mjs` ok 24214; `mock_settings.test.mjs` ok 16; `mock_index.test.mjs` ok 7 (new) |
| `python3 -B -m unittest tests/i18n/test_*.py` | test_ge0a 22 OK (2 heavy skipped; +1 `test_probes_hold`); test_glossary_context 8 OK; test_update_mock_rows 5 OK (1 skipped) |
| `TZ=Europe/Amsterdam python3 -B tests/i18n/intl_jsc.py`, `check_pairs.py`, `glossary.py` | 66 values, 0 mismatches; 0 errors, 0 warnings (144 pairs); 0 terms with pairs that lack the phrase, 1 known row |
| `python3 -B tools/ui-manifest.py`, `--check`; `python3 -B scripts/verify-ui.py` | regenerated (15 changed since the last manifest); up to date (181 files; 83 changed, 20 added against the original 161); OK. Entry `round3_english_review`: +3 files (`explorer/util.js`, `taskmgr/pages/details.js`, `processes.js`) and a fix-round sentence |
| Screenshots: `tools/heavy.sh --tag r3-ui-shots -- tests/update-ui/shoot.sh desktop.emptytrash,taskmgr-dialog.end,alles.options,alles.error,alles.properties,alles.preview,settings-check.language-EN,settings-check.restartall-dry,settings-check.restartall-sent,settings-check.recovery-link` | 20 shots, 0 failed; moved to `.rt/r3-ui/fix/shots/` (with nl/en sheets) and `tests/update-ui/shots/` removed again. Reviewed: the longer en dialog texts wrap inside the dialogs, "XCF Image" fits the properties grid, "There’s a problem with this search" fits the InfoBar, the restart button is greyed out after the real answer and usable after the dry run, the language dropdown shows "Nederlands". No clipping or overflow |

### 11.4 Left, for Robin, decisions for the coordinator (added to §8)

* **Start tiles at once (m6):** still only measurable with the GNOME harness, which writes into `tests/gnome/` (not
  R3-UI's files): §8 item 5's command on the merged build.
* **Host TSV glossary (m5):** R3-NATIVE makes check_rust's glossary findings errors and reads `glossary-context.tsv`
  for host rows (or fixes the 4 texts), so a host TSV row can no longer miss a glossary phrase silently.
* **D-G31's import rule (n4):** the decision says "the current one stays, as the mock and H do", but the old mock and
  H fell back to "nl". The mock now keeps the current one; the coordinator confirms before R3-NATIVE aligns
  `host/src/settings.rs` `import()`.
* **cross.md §7 item 9 (n6):** the extension's Ctrl+Alt+Tab group "Win11 taskbar" vs "Win11 Taskbar" needs an owner
  in `extension/`.
* **GW5 (n8):** run check.py right after the GW5 merge; the Retry row covers `update-release.js` on purpose.
* **For Robin:** n3, the restart-row description's "your PC" / "je pc" (24H2: "your device" / "je apparaat"); the
  earlier list in §8 stands.

### 11.5 Files changed in the fix round

`vendor/ui/js/`: `apps/alles/i18n.js`, `apps/explorer/{i18n,util}.js`, `apps/settings/pages/{bluetooth,update,
update-history}.js`, `apps/taskmgr/actions.js`, `apps/taskmgr/pages/{details,processes}.js`,
`apps/taskmgr/storage/explorer.js`, `core/software.js`, `desktop/strings.js`, `shell/search.js`, `mock/{index,shell}.js`
(not `mock/update.js`); `vendor/ui-sha256.json`, `vendor/ui-patches.json`; `tests/i18n/`: `dump.html`,
`surfaces.json` (+5 and a `_comment` line), `baseline-{nl,en}.txt`, `BASELINE-CHANGES.ui` (+1 line), `MOCK-CHANGES.md`,
`test_ge0a.py`, new `mock_index.test.mjs`; this note.
