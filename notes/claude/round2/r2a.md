# R2-A: system and settings, Rust texts to the catalogue (2026-11-05)

W = `.wt/r2-a` (branch `r2/a`, from d666df54). Nothing committed. No win11-* binary, gnome-shell, installer,
gsettings or dconf ran outside the sandbox. Every cargo run went through `tools/heavy.sh` (tags `r2a-check`,
`r2a-test`).

Before I started: `git -C W status --short` was empty and HEAD was d666df549249c1ac9cde3768ae1c39729cede9f8.

## 1. Result

| Check | Result |
|---|---|
| Sites converted | 106 call sites in 14 files to 90 catalogue rows in 8 stems. 104 went through `extract_rs.py convert`, 2 are D-G2 rows written with `extract_rs.py add`. There are also 3 English halves of inline pairs (`map --en`). |
| Bluetooth key | 16 Bluetooth ApiErrors now carry `key: "system.<strings.json entry>"`, so the UI's `system.bt_not_responding` match works. Codes are unchanged. |
| Dutch changes | 2, both D-G2: `system.nightLight.unsupported` and `display.unsupported`. Each is now the UI's own D-G2 text. One technical literal was removed (`"nl"`). |
| Drift and ratchet | 0 errors, 0 warnings |
| check_rust | 0 errors, 0 warnings |
| cargo check | lib 28, win11-launch 2 (the baseline). No new warning in my files. |
| Tests | lib 284 passed (271 + 13 new), 8 ignored. Oracles 16/16. `guard-calls.log` is 0 bytes. |
| Extras | All 5 are done (section 5). |

## 2. What I converted, per file

Key names follow `<stem>.<meaning>`. Where H had a good name I used it. Every en row uses the typographic ’ (D-G23).
Where the converter offered H's en (exact nl match), I took it with ’ instead of '. Every converted service site
reads `self.ctx.lang()`/`ctx.lang()`. Free functions that had no context now take a `lang: Lang` parameter, and
their callers pass `self.ctx.lang()`. The store's own language is `SettingsStore::lang()` = `language_in(config)`.

| File | Sites | Rows (stem) | Notes |
|---|---|---|---|
| services/system.rs | 3 | `system.invalidValue` | `number_arg`/`pct_arg`/`bool_arg_exact(v, lang)`. The oracle test passes `Lang::Nl`. New `text_error()`, see extra 4. |
| services/system/audio.rs | 6 | `system.audio.{noDevice,volumeFailed,muteFailed,unknownDevice,deviceGone,switchFailed}` | `Audio::set(args, lang)` and `set_sink(id, lang)`. Bluetooth's audio hand-off passes `bt.ctx.lang()`. |
| services/system/hardware.rs | 4 | `system.brightness.{unsupported,missing,failed}` | `brightness_set(args, lang)` |
| services/system/network.rs | 7 | `system.network.{wifiFailed,internalError,noAdapter,disconnectFailed,badName,badPassword,airplaneFailed}` | Codes are unchanged. `bad_args` still opens the password field (quicksettings.js). The Wi-Fi texts stay in strings.json (section 6). |
| services/system/power.rs | 7 + 1 D-G2 | `system.power.{noSession,unknownAction,inhibited,denied,failed}`, `system.updates.noApp`, `system.nightLight.{failed,unsupported}` | `open_updater(lang)`, `NightLight::set(.., lang)`. The night-light poll passes `s.ctx.lang()`. |
| services/system/energy.rs | 13 | `system.energy.*` (13 rows) | `Preferences::set(args, bus, lang)`, `write(.., lang)`, `Patch::parse(args, lang)`, `validate/set(args, lang)`. energy/tests.rs passes `Lang::Nl` (22 call sites, mechanical). Wording follows the UI's "Power mode". |
| services/system/bluetooth.rs, agent.rs | 14 + 2 | none (strings.json) | `ApiError::new(code, &text(ctx, entry))` became `text_error(ctx, code, entry)`, with the same code and text plus `key`/`params`. The unused `ApiError` import in agent.rs is gone. |
| services/display.rs | 7 + 1 D-G2 | `display.{unknown,gone,badMode,modeUnsupported,badScale,scaleRange,failed,unsupported}` | The monitors.conf header stays (C9). |
| services/media.rs | 5 | `media.{unknownControl,unknownPlayer,playerClosed,notResponding}` | |
| services/notifications.rs | 5 + 3 pairs | `notifications.{invalid,invalidAction,panelOnly,gone}`, `notifications.test.{summary,body,ok}` | The `if nl {…} else {…}` pairs were rewritten by hand: `test_notification(lang, silent)` now builds the test item. The English halves are recorded with `map --en` (frozen #178, #181, #187). `"nl"` (#170) is gone (BASELINE line). |
| services/shell.rs | 7 | `shell.{unknownCommand,unknownApp,shuttingDown,openFailed,badArgs,tooMuchData,userFallback}` | `bad_args(lang)`, `clean_params(v, lang)`, `user(lang)`. `"gebruiker"` is the user-name fallback in shell.info (en "user", lower case like a login name; now `unwrap_or_else`). backlog.rs calls `bad_args(self.ctx.lang())`. |
| services/shell/clipboard.rs | 2 | `clipboard.{notFound,pinLimit}` | `History::pin(id, pinned, lang)`. The oracle test passes `Lang::Nl`. |
| services/shell/filemanager.rs, params.rs, backlog.rs | 0 | `filemanager1` stays empty | No user-visible Dutch. FileManager1's D-Bus errors are English protocol text for the calling app, and its policy reasons are diagnostics. |
| services/tray.rs | 19 | `tray.*` (11 rows) | `point(args, lang)` |
| services/tray/helpers.rs, icons.rs | 0 | | `"onbekend"` is allow-listed (section 4) |
| settings.rs | 15 + 2 new D-G24 sites | `settings.*` (12 rows) | Section 5. `error_data(code, message, data)` became `with_data(ApiError::tr(…), data)`, so bad-value/bad-path/save errors carry key+params. The two io-error texts (`dirNotOwned`, `lockNotPrivate`; they end up in save_failed's `data`) are rendered with the store's language. `settings.exportName` takes `{date}` (`{}`=date). |
| settings_runtime.rs | 1 | `settings.stopping` | `self.store.lang()` |
| settings_effects.rs | 0 | | The gtk.css markers stay (C9). `"{} ontbreekt"` is allow-listed. |

**Refused or hand-done cases:**
* **The 2 D-G2 rows** (`add`, `#@ new`): I took nl and en programmatically from `vendor/ui/js/apps/settings/pages/display.js`
  (`nightNA`, `displaysNA`), so they are byte-identical to the UI. The old literals are listed in
  BASELINE-CHANGES.rust-r2a.
* **The 3 notification pairs**: convert for the nl half, the `if` rewritten by hand, then `map --en` for the English half.
* **No format specs, no `{{word}}`, no plurals** (no D-G1a case in my files).

## 3. Every Dutch change (tests/i18n/BASELINE-CHANGES.rust-r2a)

| Key | Old nl | New nl | en | Reason |
|---|---|---|---|---|
| `system.nightLight.unsupported` (power.rs#73) | Nachtlampje werkt hier niet. Daarvoor zijn Hyprland en hyprsunset nodig. | Nachtlampje kan hier niet worden ingesteld. Gebruik daarvoor de app Instellingen van Ubuntu. | Night light can’t be set up here. Use Ubuntu’s Settings app instead. | D-G2, = UI nightNA |
| `display.unsupported` (display.rs#17) | Beeldscherminstellingen wijzigen kan alleen in de Win11-sessie. | Beeldscherminstellingen wijzigen kan hier niet. Gebruik daarvoor de app Instellingen van Ubuntu. | Display settings can’t be changed here. Use Ubuntu’s Settings app instead. | D-G2, = UI displaysNA |

In both cases the behaviour is unchanged: the same condition and code, only the message. The file also has one
technical line: notifications.rs#170 `"nl"`, which was removed together with the pairs; the Dutch output is unchanged.
No other text in my files names Hyprland, hyprsunset or hyprctl, or says "alleen in de Win11-sessie" to the user. The
filemanager policy reasons ("layer mode in Hyprland") are English diagnostics, and the oracle pins them.

## 4. Allow-list rules (tests/i18n/allow-host-r2a.txt)

| Rule | Reason |
|---|---|
| `tray/helpers.rs fn:item_key&text:onbekend` | the persistent key `sni:onbekend` in settings.json `taskbar.tray.known`; translating it would forget remembered icons |
| `system/bluetooth.rs fn:get&text:Alias` | the BlueZ property name `Alias`; the classifier called it English user text |
| `settings_effects.rs fn:gtk4_add&text:{} ontbreekt` | an io::Error text that `sync_gtk4_block` discards (`Err(_)` → `action: "error"`); never shown or logged |
| `settings.rs fn:patch&text:language` | the setting path in the D-G24 error's data; the classifier pairs it with `"nl" \| "en"` |

## 5. The extras

**1. Defaults on every load and reload** (settings.rs):
* New `effective(value) = deep_merge(defaults, sanitize(value))`. It is the start-up rule, and now applies to every
  `read_current`: start-up, `snapshot`, `reload` (the 150 ms poll in settings_runtime.rs:83) and `adopt_disk_exact`
  (cross-host invalidations; its flag now only bypasses the byte cache, renamed `reread`).
* A load never writes. The file keeps what its writer saved.
* For consistency, a native write also returns, caches, observes and emits `effective(new)`, while it still writes
  `new` byte-identical to before.
* As a result, the writer, its peers and a restarted host all see the same document. In 3b898e6 only a restart
  merged the defaults, so a live peer saw a partial subtree that a restart did not.
* A write that does not change the merged view is now a no-op: no file write and no event. Example: replacing
  `theme` with `{"mode":"dark"}` when the mode was already dark.
* Two existing tests encoded the old partial view. I rewrote them with the new expectations, and the file-content
  assertions stay:
  * `replacing_a_subtree_writes_it_as_given_but_every_view_keeps_the_defaults`
  * `an_unknown_descendant_of_a_known_scalar_is_written_but_every_view_shows_the_default` (now on
    `theme.mode.custom`, because `language.custom` is refused by D-G24)
* New tests:
  * `a_partial_file_keeps_every_default_on_load_and_reload_and_is_not_written_back`: `{"language":"en"}` alone. An
    older writer's `{"language":"nl","theme":{"mode":"light"}}` gives changed = [language, theme.mode] only. The
    file is byte-unchanged.
  * `unknown_keys_survive_and_nested_objects_are_merged_on_every_reload`
  * `the_object_form_of_language_reads_as_dutch_on_every_load`: the view and `i18n::language_in` both give nl, and
    the file is untouched.
* The Dutch output is unchanged.

**2. D-G24** (settings.rs `set` and `patch`):
* A language string other than exactly `nl`/`en` is refused, and so is a path below `language` (the object form).
  The error is `invalid_value`, data `{"path"}` / `{"paths":["language"]}`, with the existing catalogue text
  `settings.badValue` ("Deze waarde past niet bij deze instelling." / "This value isn’t valid for this setting."),
  the mock's text. So there is no new Dutch.
* A wrong type (number, null, object) stays `bad_value`, because the type check runs first as before.
* `import` and reading stay C1: nothing new is refused there, and any other value reads as nl.
* Test: `the_language_is_set_to_exactly_nl_or_en_and_anything_else_is_refused`.

**3. D-G2:** section 3.

**4. Bluetooth key:**
* `text_error(ctx, code, entry)` (system.rs) builds the error from strings.json and sets `key = "system.<entry>"`
  (the table's own name, as H named its rows) with `params = {}`.
* set_powered's stuck-adapter error is therefore `{"code":"not_ready", …, "key":"system.bt_not_responding"}`. That
  matches `bluetooth.js:233` and `quicksettings.js:939`.
* All 16 Bluetooth ApiErrors use it. The `{"ok":false,"error":…,"code":…}` result objects (connect/pair) are
  unchanged, and the UI matches those on `code`.
* I grepped vendor/ui (non-mock) for every `.code`/`.key` match:
  * `not_ready`, `not_responding`, `needs_password`, `wrong_password`, `bad_args` (Wi-Fi password field),
    `unknown_method`, `unsupported` (display/night light/clipboard), `not_found` (tray menuEvent), `cancelled`,
    `stale`, `failed` (Bluetooth reset).
  * Every code my services return is unchanged. A script compared every removed and added code in the diff: the only
    new code is `invalid_value` (D-G24).

**5. Notes only:**
* Software* is untouched.
* `WIN11_GNOME_CONFIG_DIR` (settings_runtime.rs:40, runtime.rs, backend.rs, apps/mod.rs, files/mod.rs, i18n.rs) is a
  test/dev-only override of the host's config dir. The extension never reads it (C1: GLib's user config dir only), so
  with it set, host and extension could disagree on the language. The GNOME harness refuses to run with it set (F24
  guard `gnome-config-dir-set`). I made no code change.

## 6. Verification (from W's root, final state)

| What | Command | Result |
|---|---|---|
| Drift, my paths | `python3 -B tools/i18n/drift_rust.py --paths <the 25 non-test .rs files of my allowlist> --warnings` | 13450 frozen, 3289 present, 170 mapped or listed, 117 rows, 123 new; **0 errors, 0 warnings** |
| Drift + ratchet | `python3 -B tools/i18n/drift_rust.py --warnings` | 13280 present, 170 mapped or listed (GE1b: 60), 117 rows, 294 new; **0 errors, 0 warnings**. `--new` lists one non-technical new literal of mine, settings.rs:194 `"language"` (bilingual-pair/en, allow-listed). No `new:` warning. |
| Catalogue | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` | 24 stems, 117 rows; **0 errors, 0 warnings** (the label rule ran with node) |
| All layers | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check.py` | **0 errors**, 52 warnings (all glossary warnings of the UI layer, until round 3; none from rust) |
| Build | `tools/heavy.sh --tag r2a-check -- cargo check --offline --locked -j2 --all-targets` | 0 errors. **lib 28, win11-launch 2** (baseline); lib test 26, win11-launch test 1. The warnings in my files are all pre-existing: tray.rs 14/836/1009, tray/icons.rs 4/6, shell.rs 76, clipboard.rs 10, system/common.rs 171, settings_effects.rs 77. None is on a line I added. |
| Tests | `tools/heavy.sh --min-mb 2048 --tag r2a-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | rc 0. **lib 284 passed** (271 + 13 new), 8 ignored; win11-launch 5 passed, 1 ignored. Oracles **16/16**: shell oracle_tests 8, disks python cases 7, system reference_contracts 1. `every_converted_site_renders_its_3b898e6_dutch` ok. `logs/guard-calls.log` **0 bytes**. 13.95 s wall, 954,504 kB max RSS. |
| en render per stem | part of the run | system: `system::i18n_tests` (3 tests). display, media, notifications, tray: one each. shell+clipboard: `shell::tests::shell_and_clipboard_errors_render_in_the_language_of_the_settings_file`. settings: `settings_errors_render_in_the_language_of_their_settings_file`. filemanager1: no rows, no test. |
| Frozen text | grep of my files for `OnceLock`/`static`/`lazy`/`thread_local` | none holds rendered text: strings.json table (looked up per language), HTML entities, settings defaults, h24 flag, pairing agents. No cache I touched holds rendered text. |
| Status | `git -C W status --short` | only my files: 21 .rs, 8 .tsv, `BASELINE-CHANGES.rust-r2a`, `allow-host-r2a.txt`, this note |

The first test run failed 2 tests:
* My own wrong expectation: a peer's `changed` also lists `language`.
* `patch_reset_import_and_export_keep_original_shapes_and_order`, which pinned the Dutch export name although the
  test imports `language: "en"`. That is a test pinning Dutch in an English context (gate §5), so I updated it to
  `win11shell-settings-`. The Dutch name is checked in the settings en/nl test.

## 7. Deviations

1. **Bluetooth keys are not TSV rows.** D-G8 keeps strings.json, so `system.bt_*` names the JSON entry and check_rust
   cannot see these keys. `bluetooth_table_errors_carry_the_key_the_ui_matches` checks that every `bt_` entry has
   nl and en and that the key is `system.<entry>`.
2. **D-G24 code `invalid_value`** follows the task and plan.md. The mock (frozen) answers `bad_value` for the same
   input; the message text is the same. I also applied it to `settings.patch` (otherwise a patch bypasses it). Import
   is unchanged.
3. **Extra 1** goes further than "loads": the writer's own result and cache are the merged view too (section 5). This
   changed two existing tests.
4. **Shell oracle helper.** `services/shell/tests.rs` `result()` drops `key`/`params` before comparing with
   reference-tests.json, after asserting that key + params re-render the message. Without that, every converted error
   fails the oracle, because the JSON predates C2. The oracle JSON is untouched.
5. **Plan files.** The gate says to keep them in the notes directory, but my allowlist is `r2a*.md`, so I kept them in
   the session scratchpad. Another agent apparently cleaned that shared scratchpad. The plans' content (key, en,
   translator note) is in the rows' comments.
6. While diffing warnings I ran `touch host/src/lib.rs`, which changed only its mtime (no content change; git shows
   nothing).

## 8. Left

* strings.json's Bluetooth and Wi-Fi texts stay JSON (D-G8). The Wi-Fi texts were already bilingual and are unchanged.
* The test notification is rendered once and kept like every notification (content keeps its language). Not keyed.
* Code paths for Hyprland/hyprsunset/hyprctl are unchanged (behaviour). Under a Hyprland session without hyprsunset,
  the night-light message would now point to Ubuntu's Settings; the GNOME version never runs there.
* No GNOME harness scenario (optional). Nothing here changes the UI, and the UI shows its own D-G2 notes for these two
  cases.
* cross.md item 6 (a dropdown value that is not an option for a hand-edited language) stays round 3 UI. Reading is
  C1, and `settings.get` still returns such a string.

## 9. Decisions for the coordinator

1. Bluetooth key scheme `system.<strings.json entry>` (implemented). If strings.json later moves into system.tsv,
   H's row names are the same keys.
2. D-G24: either align the mock to `invalid_value` in round 3, or switch the host to `bad_value`. Also decide whether
   `settings.import` should drop an invalid language (the mock's sanitize does).
3. Extra 1 semantics: every view is the merged view, including the writer's, and a write that does not change the
   view is a no-op (implemented).
4. The shell oracle helper change (deviation 4). R2-B's disks oracles may need the same if they compare `e.json()`.
5. `WIN11_GNOME_CONFIG_DIR`: document it as test/dev-only in README.en (round 3), or remove it.
6. Merge `allow-host-r2a.txt` (4 rules) and `BASELINE-CHANGES.rust-r2a` (2 key lines and 3 literal lines).
7. The session scratchpad appears to be shared by the round-2 agents (other agents' files appeared in it and mine
   vanished). Give each agent its own scratch directory.

## 10. Fix round (after the review, 2026-11-05)

Starting state: `git -C W status --short` and `git -C W diff --stat` matched the review's view (29 changed files, the
same 8 stems and 21 .rs files, plus the two lists and this directory). Nothing else had changed. HEAD is still
d666df54. Nothing is committed. Every cargo run went through `tools/heavy.sh` (tags `r2a-check`, `r2a-test`).

### Per finding

| Finding | What I did |
|---|---|
| m1 test notification stored rendered | **Not changed, with a reason.** The coordinator should record it as an exception to gate §2. (1) On GNOME the live path is the extension: notifications.rs passes `language` to `notificationsBridge.js` `notifications.test`, which renders the text once into a GNOME `MessageTray.Notification`. GNOME cannot re-render that, and `extension/**` is frozen. The host's own store is only the fallback when the extension does not answer. If only the fallback re-rendered, the two paths would behave differently. (2) Windows 11 and GNOME keep the text a notification arrived with. Every other item in the same store (D-Bus `Notify` from apps) is content in its sender's language. (3) The `notifications.new` event and the UI toast already show the text rendered at emit time, which gate §2 allows. The code comment at `test_notification` says this. |
| m2 "is not responding" | **Not changed: the review's fix conflicts with the binding glossary.** I applied it first (`extract_rs.py en … isn’t responding`), and `check_rust.py --warnings` then gave 2 new `[glossary]` warnings: `tools/i18n/glossary.tsv:87` maps `Reageert niet` to `Not responding` (Windows 11's own term: Task Manager status, the "(Not responding)" title). Round 3 turns glossary warnings into errors. The UI also uses "is not responding" in its hang texts: `taskmgr/actions.js:76` hangTitle, `hostbanner.js:25` downTitle, `hostcall.js:42` timeout and `storage/explorer.js:32` hostDown. "isn’t responding" appears only in its Bluetooth titles. strings.json `bt_not_responding` also says "Bluetooth is not responding.". So I put both rows back with `extract_rs.py en`. The tray en test now pins "This app is not responding.". |
| m3 API-argument wording | **Fixed** with `extract_rs.py en … --apply`: `system.brightness.missing` "Specify a brightness level.", `system.energy.badPatch` "Specify exactly one valid power setting.", `settings.noPath` "Specify which setting to change.", `system.updates.noApp` "Couldn’t find an app for updates.". I used "Specify" for the three "Geef … op/aan" rows rather than the review's "No … was given" / "Give …" / "Choose …". It is the plain English for *opgeven*, it does not imply a text field, and it gives the three rows one pattern. "één" becomes "exactly one". The two tests that pinned the old en were updated (`system::i18n_tests`, `settings_errors_render_in_the_language_of_their_settings_file`). No glossary warning. |
| m4 test gaps | **Fixed.** The D-G2 errors are now one small fn each: `power::NightLight::unsupported(lang)` and `display.rs` `unsupported(lang)`. The call sites (`NightLight::set`, `Display::set`) return them unchanged (same code, key and text). The tests assert both on every machine: the full JSON in en, the message in nl, and equality with the UI's own `nightNA`/`displaysNA`. The test reads `vendor/ui/js/apps/settings/pages/display.js` through the new test helper `system::common::ui_display_text`. It returns None only outside the repository (source tarball), and it fails when a name is not there exactly once per language. The call-site check under `!supported()`/`!hypr_available()` stays, and it now compares the whole error. Tray: the new test `lookup_errors_render_in_the_context_language_through_call` uses a Tray struct without a D-Bus thread (like display's test). It checks `tray.invalidIcon` and `tray.iconGone` through `call()` → `find()` in en and nl, and the old test pins `tray.notResponding` too. |
| n1 D-G24 code | Coordinator decision (unchanged: host `invalid_value`, mock and H `bad_value`, same text). I documented the split in the D-G24 test: `set("language", {…})` and `patch({"language":{…}})` stay `bad_value` (data `{"path":"language"}` / `{"paths":["language"]}`), because the type check comes first. Only a dotted path below `language` (`language.custom`) gets `invalid_value`. |
| n2 import keeps "fr" | Coordinator decision (decision 3 of §9); harmless under C1. No change. |
| n3 bt keys | Accept the scheme (decision 1). No change. |
| n4 flight mode | No host change: the host follows the binding glossary, and round 3's glossary lock-in fixes the UI side. |
| n5 adopt_disk_exact | **Renamed** to `reread_disk`, in settings.rs (fn and tests) and settings_runtime.rs:68. No other code uses the name. Two coordinator notes still say `adopt_disk_exact`: `notes/claude/gnome-map.md:390` and `notes/claude/map/ui-shell.md:120`. I do not own them. |
| n6 bluetooth reset.error | No change (short-lived, the same reset). |
| n7 extra-1 semantics | Coordinator accepts (decision 3). No change. |
| n8 process / rustfmt | Every one of the 21 changed .rs files was rustfmt-clean at 3b898e6 and at d666df54. I checked this with `git show <rev>:<file> \| rustfmt --edition 2024 --emit stdout`, which gave 0 diff lines for each file. The step's one-line call sites broke that. I ran rustfmt on each file through stdin (so it does not follow `mod` declarations into files I do not own). That only reflows the changed hunks: deletions went from 435 to 404, because the baseline's multi-line calls came back. Now all 21 files are rustfmt-clean again. No string literal or literal ordinal changed (rustfmt only moves whitespace, line breaks and trailing commas), and drift, check_rust and the review's mapping scripts give the same results. The frozen pilot files (runtime.rs, window.rs, i18n.rs) are not rustfmt-clean either; that is the coordinator's call. |

### Changed in this round

* Catalogue rows (only through `extract_rs.py en`): system.tsv (3 en), settings.tsv (1 en). media.tsv and tray.tsv
  were changed and then put back, so they are byte-identical to the reviewed state.
* Code: `services/system/power.rs` (`NightLight::unsupported`), `services/display.rs` (`unsupported` + test),
  `services/system/common.rs` (test helper `ui_display_text`), `services/system.rs` (i18n test),
  `services/tray.rs` (new test, pin), `settings.rs` (rename, D-G24 test cases, test text), `settings_runtime.rs`
  (rename). rustfmt: all 21 changed .rs files.
* No Dutch text changed. BASELINE-CHANGES.rust-r2a and allow-host-r2a.txt are unchanged (5 lines, 4 rules; the
  `fn:patch&text:language` rule still matches, now at settings.rs:212).

### Done-when (from W's root, final state)

| Check | Command | Result |
|---|---|---|
| Drift, my paths | `python3 -B tools/i18n/drift_rust.py --paths <the 25 non-test .rs files> --warnings` (list in `.rt/r2a-fix-paths.txt`) | 13450 frozen, 3289 present, 170 mapped or listed, 117 rows, 123 new; **0 errors, 0 warnings** |
| Drift + ratchet | `python3 -B tools/i18n/drift_rust.py --warnings` | 13280 present, 170 mapped, 117 rows, 294 new; **0 errors, 0 warnings**. `--new`: the only non-technical new literal from my files is settings.rs:212 `"language"` (allow-listed). No `new:` warning to explain. |
| Catalogue | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` | 24 stems, 117 rows; **0 errors, 0 warnings** (label rule with node) |
| All layers | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check.py` | **0 errors**, 52 warnings (all UI glossary, until round 3; rust 0/0) |
| Build | `tools/heavy.sh --tag r2a-check -- cargo check --offline --locked -j2 --all-targets` | rc 0. **lib 28, win11-launch 2** (baseline); lib test 26, launch test 1. The warnings in my files are the same pre-existing ones (clipboard.rs:10, shell.rs:76, system/common.rs:171, tray/icons.rs:4/6, tray.rs:14/836/1009, settings_effects.rs:77); none of these lines is in a changed hunk (`git diff -U0`) |
| Tests | `tools/heavy.sh --min-mb 2048 --tag r2a-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | rc 0. **lib 285 passed** (271 + 14 new: the step's 13 + `tray::i18n_tests::lookup_errors_render_in_the_context_language_through_call`), **8 ignored**; win11-launch 5 passed, 1 ignored. Oracles **16/16** (shell oracle_tests 8, the 7 disks python cases, system reference_contracts 1). `every_converted_site_renders_its_3b898e6_dutch` ok. All 10 en render tests ok. `logs/guard-calls.log` **0 bytes**. 17.87 s wall, 945,016 kB max RSS. No `win11-r2a-*`/`w11-ge0b-*` temp dir left. The first attempt was killed by the heavy.sh watchdog (MemAvailable 1407 MB < 1500 MB, machine-wide memory pressure). Nothing had run yet; the retry passed. |
| en render per stem | part of the run | system (3), display, media, notifications, tray (2), shell+clipboard, settings; filemanager1 has no rows |
| Mappings | the review's `map_check.py`, `nl_vs_git.py`, `sites.py`, copied to `.rt/r2a-fix/` (own output path) | 107 mappings, 0 bad; 107/107 row texts byte-identical on their 3b898e6 line. All 88 keys keep their 3b898e6 error codes; the one new code is `invalid_value` for `settings.badValue` (D-G24). The shell keys were checked by hand, because the script's `#[cfg(test)]` split skips shell.rs. |
| Frozen text | grep of my 25 files for `OnceLock`/`LazyLock`/`lazy_static`/`thread_local`/`static` | unchanged: the strings.json table (looked up per language), HTML entities, settings defaults, H24, the agent registry, counters. None holds rendered text. No `current_language()` in my files. |
| rustfmt | `rustfmt --edition 2024 --emit stdout < f \| cmp - f` for the 21 changed .rs files | all clean |
| Status | `git -C W status --short` | only my files: 21 .rs, 8 .tsv, `BASELINE-CHANGES.rust-r2a`, `allow-host-r2a.txt`, `notes/claude/round2/` (this note and the review) |

### Decisions for the coordinator (added)

8. m1: record "a delivered notification keeps its language" as an exception to gate §2 (the extension's live path
   cannot re-render a GNOME notification either).
9. m2: the glossary's "Not responding" wins over contraction. If Robin prefers "isn’t responding", change the glossary
   row and the 4 Task Manager UI texts together in round 3, then the two host rows with `extract_rs.py en`.
10. rustfmt: my 21 files are rustfmt-clean again. Decide whether round 3 runs rustfmt on the GE1b pilot files
    (runtime.rs, window.rs, i18n.rs) and makes it a check.
