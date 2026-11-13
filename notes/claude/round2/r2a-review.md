# R2-A review: system and settings (adversarial, 2026-11-05)

W = `.wt/r2-a` (branch `r2/a`, HEAD d666df54, nothing committed). I did not write this work. I edited no product
file. The only files I wrote are this note and scratch helpers under `W/.rt/review-r2a/` (`map_check.py`,
`nl_vs_git.py`, `sites.py`, the `list --all` dump, logs). Heavy runs went through `W/tools/heavy.sh` with tags
`r2a-rv-check` and `r2a-rv-test`. No win11-* binary, gnome-shell, gsettings/dconf or installer ran.

## Verdict: pass

I found no blocker and no major. There are 4 minors (English wording, a deviation from the gate's rule for stored
texts, and test coverage) and 8 notes. Most notes are coordinator decisions the step already listed.

## 1. Checks I ran myself (from W's root)

| Check | Command | Result |
|---|---|---|
| Drift, the step's 25 non-test .rs files | `python3 -B tools/i18n/drift_rust.py --paths <25 files> --warnings` | 13450 frozen, 3289 present, 170 mapped or listed, 117 rows, 123 new; **0 errors, 0 warnings** |
| Drift + ratchet, full tree | `python3 -B tools/i18n/drift_rust.py --warnings` | 13280 present, 170 mapped, 117 rows, 294 new; **0 errors, 0 warnings**. `--new` (non-technical): the only new literal from the step's files is `settings.rs:194 "language"` (allow-listed) |
| Catalogue | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` | 24 stems, 117 rows; **0 errors, 0 warnings** (label rule ran with node) |
| All layers | `… python3 -B tools/i18n/check.py` | **0 errors**, 52 warnings (all `glossary`, UI layer, until round 3) |
| Build | `tools/heavy.sh --tag r2a-rv-check -- cargo check --offline --locked -j2 --all-targets` | rc 0. **lib 28, win11-launch 2** (the baseline); lib test 26, launch test 1. Warnings in the step's files are all pre-existing and on lines the step did not add: clipboard.rs:10, shell.rs:76, system/common.rs:171, tray/icons.rs:4/6, tray.rs:14/836/1009, settings_effects.rs:77 |
| Tests | `tools/heavy.sh --min-mb 2048 --tag r2a-rv-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | rc 0. **lib 284 passed, 8 ignored** (271 + 13 new); win11-launch 5 passed, 1 ignored. Oracles **16/16**: `shell::tests::oracle_tests` 8, the 7 disks python cases, `system::reference_contracts` 1. `every_converted_site_renders_its_3b898e6_dutch` ok. All 9 new en render tests ok. `logs/guard-calls.log` **0 bytes**. 14.34 s wall, 952,184 kB max RSS. No `win11-r2a-*` temp directory left in /tmp |
| Every #@ mapping vs the frozen table | `python3 -B .rt/review-r2a/map_check.py` | **107 mappings, 0 bad**: 104 nl plus 3 English halves (notifications.test.*). Each sha matches. For each one, the 3b898e6 error code next to the literal equals the code at the current call site (`sites.py`) |
| Row text vs `git show 3b898e6:<file>` | `python3 -B .rt/review-r2a/nl_vs_git.py` | **107/107** row texts (with `{date}` turned back into `{}`) found byte for byte on the 3b898e6 source line that the frozen table names |
| Missed sites | `extract_rs.py list` on all 27 owned files, then `list --all` (3,437 literals), filtered by class and by `tests/i18n/dutch-words.txt` | `list` shows 0 sites in every file. Non-technical literals left: only logs and `.expect` texts, plus the 4 allow-listed ones and C5's `Melding` matcher. Technical literals with a Dutch word are only `aan`/`uit` (bool input words, system.rs:530-531) and the monitors.conf header (C9). No user text was missed (section 3) |
| Ownership | `git -C W status --short`, `git -C W diff --name-only` | Only owned files: 8 stem TSVs, 21 .rs files under the step's paths, the two lists, `notes/claude/round2/r2a.md`. No frozen file changed (i18n.rs, lib.rs, window.rs, runtime.rs, services/mod.rs, files/mod.rs, api-methods.json, events.rs, Cargo.*, software*, vendor/, extension/, scripts/, core/window.tsv, tools/, the runners and the 3 oracle JSON files all unchanged). `.rt/` is ignored |

## 2. Dutch byte-identity

* All 107 mappings render the 3b898e6 bytes (table above). Only one row has a placeholder:
  * `settings.exportName` (`{}`=date).
  * Its call site passes `"date": stamp("%Y-%m-%d")`, the same value as the old `format!`.
* There are no plurals and no format specs. Every hand-converted site is correct:
  * The 3 notification pairs (`map --en` for the English halves): nl and en are identical to the old `if nl {…} else {…}`.
  * `nl = language() == "nl"` is exactly `lang != En` under C1.
  * The JSON key order of the test notification is unchanged.
* `BASELINE-CHANGES.rust-r2a` has 5 lines, all with 5 fields. Each one is allowed and has a true reason:
  * `key:system.nightLight.unsupported` + `power.rs#73` (sha `0ebf717e…`, 3b898e6 line 154, "…Hyprland en hyprsunset
    nodig."). This is D-G2. The new nl and en are byte-identical to `display.js` `nightNA` (lines 27/65).
  * `key:display.unsupported` + `display.rs#17` (sha `e55f048f…`, line 86, "…alleen in de Win11-sessie."). This is
    D-G2. It is byte-identical to `displaysNA` (lines 39/77).
  * `notifications.rs#170` `"nl"` (technical, line 513). This is a removed technical literal, not a Dutch change. GE1b
    has 33 lines like it.
* No other Dutch changed. Drift is clean.

## 3. Language correctness, caches, missed sites

* **Language source.**
  * Every ApiError::tr and tr call in the step's product code uses `self.ctx.lang()` or `ctx.lang()`, or a `lang`
    parameter that its caller fills from one.
  * The settings store uses `SettingsStore::lang()` = `language_in(config)`, the same file as `ctx.lang()`.
    settings_runtime uses `self.store.lang()`.
  * There is no `current_language()` in the step's files.
  * The background paths also use the context: the night-light poll (`s.ctx.lang()`) and the Bluetooth audio
    hand-off (`bt.ctx.lang()`, error discarded).
* **Statics.** The only `OnceLock`/`static`/`thread_local` items are these: the strings.json table (looked up per
  language in `text()`), the HTML entities, the settings defaults, `H24`, the agent registry and some counters. None
  holds rendered text.
* **Caches.** None of these holds host text: `Info.cache`, `Updates.state`, `Network.devices/saved`, `Audio.cache`,
  `Display.last`, `Media.last` and the tray items. The settings read cache holds the parsed document, keyed by its
  raw bytes.
* **Allow-list.** The 4 rules are narrow (fn plus text) and true:
  * `sni:onbekend` is only `item_key` (the tray.js ranking key, never displayed).
  * `Alias` is a BlueZ property.
  * `{} ontbreekt`: the only caller, `sync_gtk4_block` (settings_effects.rs:285-289), maps `Err(_)` to
    `{"ok":false,"action":"error"}`.
  * `language` in fn patch is a data path.
* **English-only texts in the step's files** (all pre-existing):
  * D-Bus protocol errors for other apps: notifications "Service stopped"/"Unknown method", the tray watcher errors,
    FileManager1.
  * FileManager1 policy reasons, which are diagnostics.
  * settings_effects result strings, which are not displayed.
  * daemon matchers.
  * None is shown in Win11's UI. The `filemanager1` stem is correctly empty.

## 4. UI matching and envelopes (C2)

* I grepped vendor/ui (non-mock) for every `.code`/`.key` match against these services:
  * `bluetooth.js:233` and `quicksettings.js:939`: `not_ready` / `e.key === 'system.bt_not_responding'`.
  * `quicksettings.js:874`: Wi-Fi `needs_password`/`wrong_password`/`bad_args` with a password.
  * `network.js:125`.
  * `display.js:315`: `unsupported`.
  * `clipboard.js:108` and `common.js:64`: `unsupported`/`unknown_method`.
  * `bluetooth.js:277/509/540`: `failed`, `cancelled`, `stale`.
* Codes:
  * `sites.py` gives every converted site the same code as its 3b898e6 site.
  * The 16 Bluetooth `text_error` sites keep their codes (diff read line by line).
  * `set_powered` still returns `("not_ready", "bt_not_responding")`, now with `key: "system.bt_not_responding"`, the
    exact snake_case key the UI tests.
* Envelopes:
  * `code` and `message` are kept. `data` is kept through `with_data` and the `e.data = …` sites.
  * `params` is always `{}` when `key` is set.
  * No UI code matches on message text: I grepped for `.message ===`, `.includes`, and the old Dutch texts.
* The bridge (`core/bridge.js:176`) and `taskmgr/hostcall.js:78` pass `key`/`params` through, and the UI only
  matches on them.

## 5. The extras

1. **Defaults on load and reload: done, and correct.**
   * How it works: `effective()` = `deep_merge(defaults, sanitize(file))` on every `read_current`. That covers
     start-up, snapshot, `reload` (the 150 ms poll, settings_runtime.rs:83/103) and `adopt_disk_exact`
     (settings_runtime.rs:68). Nothing writes back. `ctx.settings()` goes through `store.snapshot()`, so every
     service sees the merged view.
   * Cases I worked through in the code:
     * A partial file followed by `set` writes the full document plus the given subtree. That is what 3b898e6 did
       after its start-up merge.
     * A no-op write when the merged view is unchanged is correct, because `old` is re-read under the lock.
     * Reset-all on a file that has only unknown keys still writes, because the unknown key disappears.
     * Peer watermark: a peer's change is still reported through `observe()`.
     * The object form of language reads as nl in the view and in `language_in`.
     * Defaults contain no map-like objects (checked by script), so a merge cannot resurrect a removed entry.
     * Only the language is read raw by the extension (`extension.js:41`).
   * Tests: partial file, unknown keys, nested merge and the object form, plus the two rewritten tests. All pass.
2. **D-G24: done and tested.**
   * set and patch refuse any string other than `"nl"`/`"en"` with `invalid_value`, and the dotted path
     `language.*` too.
   * The data is `{"path"}`/`{"paths"}` and the text is the existing `settings.badValue`, so there is no new Dutch.
   * A wrong type stays `bad_value`. Import and reading are unchanged (C1).
   * See notes n1 and n2 for the mismatch with the mock and with H.
3. **D-G2: done.** Two texts (section 2). No other text in the step's files describes Hyprland, hyprsunset, hyprctl
   or "Win11-sessie" to the user. I grepped all 27 files and strings.json: what is left is code paths, the
   `desktop_hypr` name and FileManager1 diagnostics. No behaviour changed.
4. **Bluetooth key: done** (section 4). Unit test: `bluetooth_table_errors_carry_the_key_the_ui_matches`.
5. **`WIN11_GNOME_CONFIG_DIR`** is recorded as test/dev-only in r2a.md §5.5. There is no code change. Software* is
   untouched.

## 6. Findings

**m1 (minor, language). The test notification is stored as rendered text.**
* **Evidence:** notifications.rs:522-523 calls `test_notification(self.ctx.lang(), …)`, then `self.add(n,0)`, which
  keeps the item in the centre. `test_notification` (:773) renders `notifications.test.summary/body/ok` once.
* **Failure:** press "Show test notification" in Dutch, then switch to English. The centre still shows "Zo ziet een
  melding eruit" and "Prima" until you dismiss it.
* **Why only minor:**
  * The store is in memory (no persistence).
  * Windows keeps a toast's language too.
  * 3b898e6 behaved the same.
  * The step disclosed it (r2a.md §8).
* **Fix:** either the coordinator accepts it explicitly as an exception to gate §2 ("a notification is content"), or
  the item stores its keys, e.g. `"i18n":{"summary":"notifications.test.summary",…}` for `source:"test"`, and
  `list`/`get` re-render them in `ctx.lang()`.

**m2 (minor, English). The negative forms are not contracted like every other row.**
* **Evidence:**
  * `media.notResponding` = "The player is not responding."
  * `tray.notResponding` = "This app is not responding."
  * Every other en row uses ’ contractions ("isn’t", "doesn’t", "can’t"), like the UI.
* **Fix:**
  * `extract_rs.py en media.notResponding "The player isn’t responding." --apply`
  * `extract_rs.py en tray.notResponding "This app isn’t responding." --apply`

**m3 (minor, English). API-argument errors are phrased as instructions to type something (literal Dutch order).**
* **Evidence:**
  * `system.brightness.missing` "Enter a brightness." (no field exists)
  * `system.energy.badPatch` "Enter one valid power setting."
  * `settings.noPath` "Say which setting you want to change."
  * `system.updates.noApp` "No app for updates was found."
  * These only reach a user through a malformed call, so the impact is low.
* **Suggested fix** (`extract_rs.py en …`):
  * "No brightness was given."
  * "Give exactly one valid power setting."
  * "Choose which setting to change."
  * "Couldn’t find an app for updates."

**m4 (minor, test gap). Two pieces of the new tests are skipped on some machines, and one test checks no message.**
* **Evidence:**
  * In `power_network_and_night_light_errors_follow_the_context_language` (system.rs) the D-G2 night-light assertion
    is inside `if !night.supported()`.
  * In `display_errors_render_in_the_context_language` the D-G2 display assertion is inside `if !hypr_available()`.
  * So on a dev box with Hyprland and hyprsunset (H's machine), neither D-G2 row is tested.
  * The tray en test checks only that each row's en ≠ nl, not the text, apart from `tray.noMenu` and
    `tray.badPosition`.
* **Fix:**
  * Assert the D-G2 rows directly as well, e.g. `ApiError::tr("unsupported", Lang::En, "system.nightLight.unsupported", NO_PARAMS).message`
    against the UI's `nightNA`/`displaysNA` strings, read from display.js in the test.
  * Pin one more tray message.

**n1 (note, coordinator). D-G24's code and its premise disagree.**
* plan.md and the task say `invalid_value` "like the mock".
* But `vendor/ui/js/mock/settings.js:56/194/208` and H's `host/services/settings.py:79-86,216` answer `bad_value`
  (same text).
* Nothing in vendor/ui or api-methods.json matches either code, so behaviour is unaffected.
* Settle it in round 3: align the mock, or switch the host to `bad_value` (one word in 2 call sites and their tests).
* Related: the object form gives `invalid_value` only through a dotted path. `set("language", {…})` and
  `patch({"language":{…}})` give `bad_value`, because the type check comes first. That is defensible; document it.

**n2 (note, coordinator).** `settings.import` still stores a language such as `"fr"`. H drops it on import and on
read ("dropped (→ the default) when read or imported"). G reads it as nl (C1), so it is harmless. The step lists
this as decision 3.

**n3 (note).** `system.bt_*` keys name strings.json entries, not TSV rows, so check_rust cannot check them.
* The unit test covers every `bt_` entry (nl, en, key form).
* Accept the scheme (decision 1). A later move of strings.json into system.tsv keeps the keys.

**n4 (note, cross-layer).** `system.network.airplaneFailed` en is "Couldn’t change flight mode."
* That follows the binding glossary (`Vliegtuigmodus → Flight mode`, Windows 11 en-GB).
* But the UI says "Airplane mode" (`quicksettings.js:82/97/98`, `network.js:49`, `taskbar.js:56`,
  `registry.js:69/71`), so an English user sees both terms.
* The host is right. The UI's glossary warnings belong to round 3's lock-in.

**n5 (note).** `SettingsStore::adopt_disk_exact` no longer adopts the exact shape. Its doc comment was updated; its
name was not (the caller is settings_runtime.rs:68, which this step owns). A rename (e.g. `reread_disk`) is optional.

**n6 (note).** Bluetooth's `reset.error: Option<ApiError>` (bluetooth.rs:53, :909-920) is a rendered error. It goes
only to callers that join the same running reset, and it is cleared when the next reset starts. That is short-lived
and acceptable.

**n7 (note).** Extra 1 deliberately changes some observable behaviour.
* A `set` of an unknown descendant of a known scalar (`theme.mode.custom`) writes the object, but every view shows
  the default.
* The written file now follows the defaults' key order on the poll path too.
* Both match 3b898e6's start-up semantics, and tests pin them.

**n8 (note, process).** The step ran `touch host/src/lib.rs`, a frozen file. The content is unchanged and git shows
nothing. Its plan files were lost in a shared scratchpad (decision 7). Its call sites are not rustfmt-wrapped (lines
over 120 characters, as in the GE1b pilot). Nothing enforces rustfmt.

## 7. Claims of the step report checked

| Claim | Result |
|---|---|
| 106 sites → 90 rows | 90 rows (system 38, display 8, media 4, notifications 7, shell 7, clipboard 2, tray 11, settings 13, filemanager1 0); 107 `#@` mappings incl. 3 en halves |
| Dutch byte-identical except 2 D-G2 | true |
| No error code changed; only new code `invalid_value` | true |
| All en rows use ’ | true (no straight apostrophe in any en row of the 9 stems) |
| Done-when numbers | reproduced exactly (section 1) |
| Statics/caches hold no text | true, except m1 (disclosed) and n6 |
| `git status` only own files | true |
