# R2-C: the files/** texts (round 2, 2026-11-05)

W = this worktree (`.wt/r2-c`, branch `r2/c`). At the start, `git status --short` was empty and HEAD was d666df54.
Nothing is committed. No win11-* binary, gnome-shell, installer, gsettings or dconf ran. Every cargo run went
through `tools/heavy.sh` (tags `r2c-check` / `r2c-test`, logged in `tests/logs/heavy-events.log`). The one
heavy run with rc 101 was a `cargo check` that found my own type errors; the one with rc 1 was a test run with a
wrong expectation in my new test (fixed, see §5).

## 1. What was converted

All user-visible literals of `host/src/files/**` are now catalogue rows: 150 Dutch sites and the 20 English halves
of 20 inline pairs, so 170 literals. They map to 115 rows. With the one row of new Dutch text, the D-G1a `.one`,
that makes 116 rows: 93 in `files` (incl. `.one`), 23 in `files_launch`. check_rust's 143 rows are these 116 plus
core 14 and window 13. (Count corrected in the fix round, review n1.)

After the conversion, `extract_rs.py list` shows 0 sites in each of the 13 files. The only literals left are the 4
allow-listed ones (§3) and technical ones.

| File | Sites (nl + en half) | Keys (`files.` unless noted) | By hand |
|---|---|---|---|
| mod.rs | 26 | shuttingDown, badArgs, badPath, noItems, folderGone, noName, slash, reservedName, badChars, nameTooLong, thisItem, quotedName, noAccessTo, notFoundItem, noSpace, readOnly, exists, inUse, notDir, nameNotOnDrive, failedFor | `io_error` rewritten first as one `ApiError::new(code, …)` per arm (same literals, same order), so the converter could make each arm `ApiError::tr` with key and params. Its pieces `dit item` / `‘{name}’` are one row each (§4, extra 3). |
| actions.rs | 30 | itemGone; `files_launch.` appNotFound, noApp, noAppForFile, appFailed, cannotOpenShortcut, untrustedDrive, untrusted, damagedShortcut, cannotOpenLink, unsupportedLink, invalidLink, fileGone, noAppForLink, notShortcut, untrustedDriveTrust, foreignShortcut, trustFailed, noTerminal, terminalNotFound, terminalFailed, launcherMissing, launchFailed, launcherNoReply | `detail_error(code, "…", data)` became `with_data(ApiError::new(code, "…"), data)` first, literals unchanged. After conversion these errors carry key and params as well as their data (C2). `untrusted()` now builds one `ApiError` per branch. |
| archives.rs | 26 | badArchive, unsupportedArchive, unsafePaths, unsafeLinks, deviceFiles, namedGone, zipMultiDisk, zip64IndexInvalid, zipIndexInvalid, zipOffsetInvalid, zipExtraInvalid, zip64SizeInvalid, zip64OffsetInvalid, zipDuplicateHeader, zipIndexLength, archiveReaderMemory, archiveGone, archiveTooLarge, noSpaceExtract, archiveChanged, extractedSizeWrong, hardLinkMissing | The ZIP and reader diagnostics are the `data` of files.badArchive. The UI does not show them, but they are Dutch API text, so they are converted too. |
| archive_ffi.rs | 3 | libarchiveMissing, libarchiveLacks, archiveFailed | **Refused, by hand** (inside `macro_rules!` and `concat!`, and frozen in a `OnceLock`). See §4: `LoadError` plus `add --from`. |
| clipboard.rs | 8 | clipboardTooLarge, clipboardUnreadable, clipboardInvalid, clipboardNotUpdated, selectionTooLarge, badOp, nothingToPaste, clipboardNotPrivate | – |
| dry.rs | 6 + 3 en | itemGone, nameExists, unknownKind, new.folder, new.empty, new.text | The pair `match (folder, kind, nl)` became `match (folder, kind)`, plus `map --en`. |
| mutate.rs | 12 + 4 en | unknownKind, new.folder, new.empty, new.text, exists, itemGone, nameExists, renameIncomplete (`{path}`), destGone, intoItself (2 sites), copyWord | The same pair as dry.rs. kopie/Copy is rendered once per transfer. The into-itself `io::Error` marker and its comparison use the same rendered text; the comparison literal was technical, so it took `--force`. |
| places.rs | 8 + 7 en | place.desktop, place.documents, place.music, place.pictures, place.videos, place.trash, place.root, place.user | `locations()` rewritten: the rows hold Strings. "Downloads"/"Downloads" stays inline as `i18n::pick` (the same word in both languages), so none of its literals change. |
| properties.rs | 7 + 2 en | allOfTypePrefix (a piece), multipleTypes, itemGone, symlink (2 sites), dotfile, changedMeanwhile | The pairs became tr calls. "Alle van het type " + type stays two pieces (extra 3). |
| read.rs | 6 + 4 en | folderGone, notDir, thisPc, type.ext (`{ext}`; the en half mapped with `--spec '{}=ext'`), type.font, type.file | – |
| search.rs | 3 | badQuery (2 sites), badBudget | – |
| trash.rs | 14 | trashLockNotPrivate, trashUnsafe, badPath, exists, noTrash, noDeleteAccess, itemGone (2 sites), trashFailed, noItems, notInTrash, otherDrive, unknownOrigin, emptyTrashFailed.other | The plural was converted to `.other` (`{failed}=n`) and then hand-edited to `ApiError::trn(…, "files.emptyTrashFailed", failed, …)`. `.one` was added with `extract_rs.py add`; its nl is taken byte for byte from H's `files.emptyTrashFailed.one`, not typed. |
| watch.rs | 1 | shuttingDown | The 3 start-up texts are allow-listed (§3). |

**Language expressions.**
* Every call site uses the call's own language: `ctx.lang()` (new: `files::Context::lang()`), or a `lang: Lang`
  parameter passed down from it. That meant threading `lang` through `abs_in`, `paths`, `directory`,
  `validate_name`, `io_error`, the archive helpers, `Reader`/`ArchiveWriter` (a per-call field), the clipboard and
  trash locks, `Options::parse`, the watcher's send/watch/unwatch and the launch helpers.
* `crate::i18n::current_language()` is left in only three places:
  * the `abs()` wrapper that apps/mod.rs calls (§7);
  * `open_external_uri` (window.rs, whose result is ignored);
  * `Clipboard::new`, a start-up error.
* `Clipboard::event_snapshot` passes `Lang::Nl`: it drops its errors, so their language does not matter.
* `Files::call`'s two early errors (offline, bad_args) use the new `Files::lang()`. It reads the reference backend's
  settings when one is attached, else `language_in(&self.config)`.

**English.**
* en-GB in Windows 11 wording, with ’ in every new row (D-G23).
* Where the converter offered H's en (exact nl match), I used it with ’ instead of '.
* The 20 English halves of the pairs keep 3b898e6's exact English: their `#@ … en` lines require it.
* The one quoted UI name is “Allow launching”, the UI's `launchAllow` (bridge.js). The label rule passes.
* "Archive Manager" (GNOME's File Roller) and "Win11 Shell" (D-G16) are names.
* `files.archiveTooLarge` was rewritten with `extract_rs.py en`.

## 2. Dutch changes (`tests/i18n/BASELINE-CHANGES.rust-r2c`)

* **One real Dutch change, D-G1a:** `key:files.emptyTrashFailed.one`. Emptying the Prullenbak with exactly 1 item
  that could not be deleted now says "1 item kon niet worden verwijderd." (H's nl). Every other count keeps
  3b898e6's "{n} item(s) konden niet worden verwijderd." I held back the cosmetic "items" for n ≠ 1.
* **9 lines for removed technical literals.** These are not text changes: the language tests of converted pairs
  (`ctx.language == "nl"` ×6, `language == "en"` ×1) and one of the two `"empty"` match arms in dry.rs and in
  mutate.rs (×2), so 6 + 1 + 2 = 9. Each line gives its reason. (Count corrected in the fix round, review n1.)
* Every other Dutch output is byte-identical for every value. `i18n::tests::every_converted_site_renders_its_3b898e6_dutch`
  passes, and the new render tests check the 3b898e6 Dutch next to the English.

## 3. Allow-list rules (`tests/i18n/allow-host-r2c.txt`, for the coordinator to merge)

| Rule | Reason |
|---|---|
| `files/mod.rs text:Bestandsgebeurtenissen starten: {error}` | start-up error: `Err(String)` of `Files::new` → `Runtime::new` → lib.rs `run()` → stderr. The same kind as the lib.rs/runtime.rs rules. |
| `files/watch.rs fn:start&text:Mapbewaking starten: {error}` (2 literals) | start-up error (`Watcher::new` → `Files::new`), stderr only |
| `files/watch.rs fn:start&text:Mapbewaking kon niet worden gestart.` | the same |
| `files/trash.rs fn:manual_one&sha:cbeda396a2b2d938` | the `.trashinfo` file format of the freedesktop.org Trash spec (`[Trash Info]`, `Path=`, `DeletionDate=`), written into user files (H D11). GE0b called it user-visible-english. |

`check_rust.py` reports no `allow` warning, so all 4 rules match a literal.

## 4. Refused and hand-converted cases (gate §5)

* **archive_ffi.rs.**
  * The two load errors were `.into()` and `concat!("De archiefbibliotheek mist ", stringify!($name))` inside
    `macro_rules!`.
  * Their result was cached as `OnceLock<Result<Library, String>>`, which froze the text in the language of the
    first call.
  * Now the cache holds `Result<Library, LoadError>`, with `LoadError::{Missing, Symbol(&'static str)}`.
    `LoadError::message(lang)` renders the text at use time.
  * The rows were made with `extract_rs.py add --from host/src/files/archive_ffi.rs#8` / `#10`; the nl came from
    `rust-frozen-all.tsv`, not typed by hand.
  * `Library::error(handle, lang)` was converted normally.
* **Sentences built from pieces (extra 3): one row per piece, no merge, so no Dutch change line.**
  * `files.allOfTypePrefix` "Alle van het type " / "All of type " (+ type name).
  * `files.libarchiveLacks` "De archiefbibliotheek mist " / "The archive library is missing " (+ symbol).
  * `files.thisItem` "dit item" / "this item" and `files.quotedName` ‘{name}’ / “{name}”, the `{item}` of the 4
    `io_error` sentences.
  * In English, a sentence that starts with `{item}` ("this item is in use.") gets an upper-case first letter in
    `io_error` (`upper_first`, en only). nl keeps 3b898e6's "dit item is in gebruik.".
  * The path is empty, so the case is practically unreachable: no API path has an empty file name.
* **Plural (D-G1a):** see trash.rs in §1.
* **Inline pairs:** 20 English halves recorded with `extract_rs.py map <key> --from <file>#<3b898e6 ordinal> --en
  [--spec]`. The ordinals were checked in `rust-frozen-all.tsv`: files/mod.rs is shifted +2 since GE1b's ApiError
  edit, but it has no pairs.

## 5. Extras of the task

1. **`files::Context::language` (extra 1).**
   * `Files::context()` now uses `crate::i18n::language_in(&self.config)` (one stat, cached by the file's identity).
     The full `fs::read` of settings.json per call is gone.
   * `explorer.showHidden` came from the same read. It is now lazy: `Context::show_hidden()` (a `OnceLock<bool>`
     per context, no text) reads it only on first use. Only `files.list` without its own `showHidden` uses it.
   * So a `files.*` call, and apps/mod.rs's 150 ms poll of `files.context().language` (cross.md §7 item 2), cost
     one stat.
   * The `language: String` field stays: apps uses it. `Context::lang()` returns it as a `Lang`.
   * With an attached reference backend, both values still come from its `settings.get`, as in 3b898e6.
2. **Tests that pinned Dutch in an English context (extra 2):**
   * `actions::tests::malformed_uri_authority_is_rejected_before_any_handler_launch` (en context) now expects
     "This link isn’t valid." and key `files_launch.invalidLink`.
   * `trash::tests::mounted_child_preflight_…` and `separately_mounted_info_…` (en context) now expect
     "1 item couldn’t be deleted.", key `files.emptyTrashFailed` and params `{"n":"1"}`.
   * `count_params` passes n as a string. My first expectation (a number) was wrong; this was the rc 1 run.
   * The other files tests use nl contexts or check codes only.
3. **Pieces (extra 3):** see §4.
4. **XDG folder names (extra 4).**
   * Nothing on disk is renamed.
   * The fallback folder names in `places.rs` (`fallback_dir`, the 5th tuple field) are unchanged.
   * Display names are localised only where 3b898e6 already did: the `files.places` names and the new-item names.
5. **types.json / launcher-names.json (extra 5): no edit needed.**
   * All 47 entries of types.json have `[nl, en]`.
   * In launcher-names.json, `NL_NAMES` is applied only in Dutch, when the .desktop file has no Dutch `Name`.
     In English the app's own `Name=` is the English name, so an `en` would repeat it.
   * `SAME_NAME_AS_OURS` already has nl and en.
   * No nl value changed.
   * **For R2-D:** `host/src/apps/catalog-data.json` keeps exact duplicates. `nlNames` (57 entries) equals
     `NL_NAMES` entry for entry, and `twins` equals `SAME_NAME_AS_OURS` (Schijven (GNOME) / Disks (GNOME)).
6. **One en render test per stem, through the real call sites:**
   * **files:** `files::tests::file_texts_follow_the_settings_language_per_call_through_files_call`.
     * It drives `Files::call` with a service whose own config dir holds settings.json.
     * en: itemGone with key/params, noName, notDir, folderGone, noItems, the `io_error` pieces
       (“missing.txt” no longer exists., key files.notFoundItem, params item), the nameless capitalisation,
       a created "New folder", and places "Recycle Bin"/"System drive".
     * It checks the lazy `showHidden`.
     * A rename to nl makes the next call Dutch: 3b898e6's bytes, "Nieuwe map". "EN" and a missing file are nl (C1).
     * It also checks the D-G1a plural.
   * **files_launch:** `files::actions::tests::launch_errors_render_in_the_context_language_with_their_keys`.
     * It covers openWith, open, launcher type/damaged/untrusted (with “Allow launching”, key, data.canTrust),
       links (scheme, empty, missing file), trustLauncher and missing_app.
     * It also checks the Dutch for an nl context.

## 6. Verification (all from W's root, final state, after `rustfmt --edition 2024` of my 13 files)

| Check | Command | Result |
|---|---|---|
| Drift (my files) | `python3 -B tools/i18n/drift_rust.py --paths host/src/files/*.rs --warnings` | 13,450 frozen, 1,243 present, 239 mapped or listed, 143 rows, 162 new literals; **0 errors, 0 warnings** |
| Drift (tree) | `python3 -B tools/i18n/drift_rust.py --warnings` | 13,211 present, 239 mapped or listed, 143 rows; ratchet 334 new literals (bilingual-pair 6, log 4, technical 320, user-visible-dutch 3, user-visible-english 1, all from GE1b), 178 catalogue keys, 20 allow-listed; **0 errors, 0 warnings**. `--new` lists no literal of files/**: all 160 of mine are catalogue keys or technical (json param names, the second "untrusted" code). |
| Catalogue | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` | 24 stems, 143 rows; **0 errors, 4 warnings**. All 4 are `[space]`: nl and en of the two piece rows `files.allOfTypePrefix` and `files.libarchiveLacks` end in a space on purpose (extra 3, "one row per piece"). No glossary, label or en-dutch finding. |
| All checks | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check.py` | **0 errors**, 56 warnings (glossary 52, all UI, "until round 3"; rust 4 = the above) |
| Build | `tools/heavy.sh --tag r2c-check -- cargo check --offline --locked -j2 --all-targets` | rc 0. lib **28** warnings, win11-launch **2** (baseline). In my files only pre-existing ones: mod.rs `native_watch`, trash.rs `list` (no caller at d666df54), the dry.rs test import `PermissionsExt` (untouched test module). |
| Tests | `tools/heavy.sh --min-mb 2048 --tag r2c-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | rc 0. lib **273 passed** (271 + my 2), **8 ignored**; win11-launch 5 passed / 1 ignored. **16 oracle tests ok** (shell `oracle_tests::*` 8, the 7 disks python cases, `system::reference_contracts`). `every_converted_site_renders_its_3b898e6_dutch` ok. `logs/guard-calls.log` 0 bytes. 13.8 s wall, 934 MB peak RSS, no `/tmp/w11-ge0b-*` left. |
| Frozen text | `grep -nE 'static \|OnceLock\|LazyLock\|lazy\|thread_local\|cache' host/src/files/*.rs` | No rendered text in any static or cache. archive_ffi's `LIBRARY` holds `Result<Library, LoadError>` (fixed, see §4). The per-context `OnceLock<bool>` is showHidden. read.rs `NAMES`/`TYPES` hold the bundled JSON with both languages (picked per call); `COMMENTS` is keyed by (mime, lang); the icon and trash-info caches hold no text. `LINK_DEPTH` is a counter. |
| ApiError block | `diff` of `struct ApiError … impl ApiError` against d666df54 | identical (frozen block untouched) |
| Status | `git status --short` | only my files: host/i18n/files.tsv, files_launch.tsv, the 13 host/src/files/*.rs, tests/i18n/{BASELINE-CHANGES.rust-r2c, allow-host-r2c.txt}, notes/claude/round2/ (this note, plan files in `r2c-plans/`) |

## 7. Deviations

* H's English was taken where the converter offered it, but with ’ for ' (D-G23); the gate says "take en from H".
  "This file doesn't exist any more." (H, for files_launch.fileGone) became "This file no longer exists.", to match
  itemGone/folderGone.
* The ZIP/reader diagnostics are converted although only `data` carries them (GE0b called them user-visible-dutch).
* `abs(value)` keeps its signature for apps/mod.rs and uses `current_language()`. Code inside files/ uses `abs_in`.
* I ran `rustfmt --edition 2024` on my 13 files. They were rustfmt-clean at 3b898e6 and d666df54. The converter's
  long lines are wrapped now, and the ApiError block is unchanged.
* `Context::show_hidden` is a method now (its field is private); only read.rs used it.

## 8. Left, and decisions for the coordinator

1. **R2-D (apps/mod.rs):**
   * apps/mod.rs:160 (`files::abs(&json!(uri))`) is inside `fn clean_uris(value: &Value)` (:143), which has no
     ctx. Give it a `lang` parameter: `clean_uris(&args["uris"], ctx.lang())` at its caller (:112, which has
     `ctx = files.context()`), and inside it `files::abs_in(&json!(uri), lang)`. Today the effect is nil:
     `current_language()` reads the same settings file as `files.config`. (Corrected in the fix round, review n2.)
   * Drop or derive `catalog-data.json`'s `nlNames`/`twins` from launcher-names.json (exact duplicates, extra 5).
   * The 150 ms language poll now costs one stat (cross.md §7 item 2, files side done).
   * `crate::launch::command`'s error texts (launch.rs) reach `files_launch` paths as `launch_failed` messages.
     They are R2-D's.
2. **Merge** `tests/i18n/allow-host-r2c.txt` (4 rules) and `tests/i18n/BASELINE-CHANGES.rust-r2c` (1 Dutch line,
   9 technical lines).
3. **`[space]` warnings:** keep them (my default; the pieces need the space), or give check_rust a way to mark a
   piece row.
4. **D-G1a scope:** the singular only. H's "{n} items konden …" for n ≠ 1 (dropping "(s)") is cosmetic and held
   back. Offer it to Robin with D-G1b if wanted.
5. **Not verified:** no GNOME harness run. The task did not ask for one, and the UI shows these texts only as
   `message`. The mock UI (vendor/ui/js/mock, frozen) still has its own texts.

## 9. Fix round (after the review `r2c-review.md`, verdict pass: 0 blockers, 0 majors, 4 minors, 6 notes)

**Start state.** `git status --short` and `git diff --stat` matched the review's view: the 13 files/*.rs, the 2
stems, the 2 tests/i18n files and notes/claude/round2/ (now also r2c-review.md). Nothing else had changed.

### Per finding

| # | What I did |
|---|---|
| m1 | **Fixed.** archives.rs: `library(lang)` maps the cached `LoadError` through a new `load_error(error, lang)`. It builds `ApiError::tr("unsupported_archive", lang, "files.libarchiveMissing" / "files.libarchiveLacks", NO_PARAMS)`, one arm per kind with literal keys, so check_rust sees both. Then it sets `message = error.message(lang)`: for `Symbol` the key names the first piece and the message is the whole sentence (piece + symbol, 3b898e6's bytes in nl). The params stay `{}`, because a param without a placeholder is a check_rust `[params]` warning. New test `archives::tests::libarchive_load_errors_render_in_the_call_language_with_their_key`: code, en message, key and params `{}` for both kinds, plus nl `Missing` and `Symbol("archive_read_new")`, byte-equal to 3b898e6's literal and `concat!` output. |
| m2 | **Fixed**, with `extract_rs.py en <key> "…" --apply` (only the en column changes). `files_launch.untrusted`: "This shortcut isn’t trusted yet, so it was blocked for your security. If you trust its source, choose “Allow launching”." The label is unchanged, so the label rule passes. `files_launch.untrustedDrive`: "For your security, shortcuts on this drive can’t be started." `files.failedFor`: "Something went wrong with {item}." `files.unknownKind`: "Unknown file type." `files.badBudget`: "Invalid search time limit." The launch test now checks the full untrusted en message: its old `ends_with("Then choose …")` matched the old wording. |
| m3 | **Fixed**, with `extract_rs.py en`. `files.noSpace`: "There isn’t enough space on the drive." `files.noSpaceExtract`: "There isn’t enough space on the drive to extract this archive." So "schijf" is "drive" in all 8 rows now. One deviation from the review's text: "the drive", not "this drive". The nl says "de schijf", and ENOSPC is about the destination's drive, which is not always the one the user is looking at. |
| m4 | **Fixed** with asserts in the existing tests, all through the real builders. `files::tests::file_texts_follow_…_through_files_call` (`Files::call`): `bad_args` for `json!([])` gives "Invalid input." with key files.badArgs; `offline` while `jobs.0` is set gives "The file service is shutting down." with key files.shuttingDown (both via `Files::lang()`); a two-file `files.properties` gives "All of type Text Document"; file + folder gives "Multiple types". After the switch to nl, the same calls give "Ongeldige invoer.", "Alle van het type Tekstdocument" and "Meerdere typen" (3b898e6 bytes). `trash::tests::malformed_info_and_missing_restore_…` (en context): failed[].error "The original location is unknown." / "This item is no longer in the Recycle Bin.", and bad_args "Select one or more items first.". `mutate::tests::transfer_reports_partial_success_…` (en): failed[0].error "“missing” no longer exists.". `mutate::tests::descendant_destination_through_a_link_is_rejected` (en): failed[0].error "You can’t put a folder inside itself." (the into-itself marker). `LoadError`: see m1. |
| n1 | **Fixed** in §1 and §2 of this note. 115 mapped rows + 1 new `.one` = 116 rows (files 93 incl. `.one`, files_launch 23), and 143 = 116 + core 14 + window 13. The technical lines are "nl" ×6 + "en" ×1 + "empty" ×2 = 9. |
| n2 | **Fixed** in §8 item 1: R2-D adds a `lang` parameter to `clean_uris` (apps/mod.rs:143) and passes `ctx.lang()` from its caller (:112), then calls `files::abs_in(&json!(uri), lang)` at :160. |
| n3 | **Fixed.** mod.rs `Files::call`: the shutting-down branch drops the `jobs` guard before it renders (`drop(jobs)`, then `self.lang()`). So the reference backend's `settings.get` no longer runs under the lock. Normal calls are unchanged: no extra stat or HTTP call per call. |
| n4 | No change (the review's advice). The UI shows `message` (C2), and a nameless path is practically unreachable. |
| n5 | Kept: the 4 `[space]` warnings are the two piece rows (extra 3). Decision 3 of §8 stands. |
| n6 | Kept: `i18n::pick(lang, "Downloads", "Downloads")` (the review's advice). A plain literal would cost 2 technical BASELINE-CHANGES lines. |

**Dutch.** No nl value changed in the fix round. BASELINE-CHANGES.rust-r2c and allow-host-r2c.txt are unchanged:
1 D-G1a line + 9 technical lines, and 4 rules.

**Files changed in the fix round:** host/src/files/{archives,mod,trash,mutate,actions}.rs (code: m1, n3; tests: m1,
m2, m4), host/i18n/files.tsv (5 en: failedFor, unknownKind, badBudget, noSpace, noSpaceExtract), host/i18n/
files_launch.tsv (2 en: untrusted, untrustedDrive), this note. rustfmt `--edition 2024 --check` is clean on the 5 .rs
files. The frozen ApiError block (`struct ApiError` … end of `impl ApiError`) has the same md5 as at d666df54
(a13dd2a8…).

### Done-when (all from W's root, final state)

| Check | Command | Result |
|---|---|---|
| Drift (my files) | `python3 -B tools/i18n/drift_rust.py --paths host/src/files/*.rs --warnings` | 13450 frozen, 1243 present, 239 mapped or listed, 143 rows, 165 new literals; **0 errors, 0 warnings**. The 3 extra new ones (162 → 165) are all in archives.rs `load_error`: the keys `files.libarchiveMissing` / `files.libarchiveLacks` (catalogue keys, which moved from archive_ffi.rs's `message` into the envelope too), and a second `"unsupported_archive"` code (one per arm). `extract_rs.py list --all` classes all 3 technical. |
| Drift (tree) | `python3 -B tools/i18n/drift_rust.py --warnings` | 13211 present, 239 mapped or listed, 143 rows; ratchet 337 new literals (bilingual-pair 6, log 4, technical 323, user-visible-dutch 3, user-visible-english 1), 180 catalogue keys, 20 allow-listed; **0 errors, 0 warnings**. `--new` lists no files/ literal: its non-technical entries are GE1b's (i18n.rs, window.rs), as before. |
| Catalogue | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` | 24 stems, 143 rows; **0 errors, 4 warnings**: the same 4 `[space]` on `files.allOfTypePrefix` and `files.libarchiveLacks` (nl and en; piece rows, n5). No glossary, label, params or en-dutch finding for the 7 new en texts. |
| All checks | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check.py` | **0 errors**, 56 warnings (glossary 52, all UI; rust 4 = the above) |
| Build | `tools/heavy.sh --tag r2c-check -- cargo check --offline --locked -j2 --all-targets` | rc 0. lib **28**, lib test 26, win11-launch **2** (baseline). In my files only the 3 pre-existing ones: dry.rs:106 (test import), mod.rs:369 `native_watch` (was :367; +2 lines from n3), trash.rs:230 `list`. |
| Tests | `tools/heavy.sh --min-mb 2048 --tag r2c-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | rc 0. lib **274 passed** (271 + my 3: the 2 render tests and the new libarchive test), **8 ignored**; win11-launch 5 passed, 1 ignored. **16 oracle tests ok**: `oracle_tests::*` 8, the 7 disks python cases, `system::reference_contracts` 1. `i18n::tests::every_converted_site_renders_its_3b898e6_dutch` ok. Both en render tests and every test touched in this round are ok. `tests/rust-sandbox/logs/guard-calls.log` 0 bytes. 20.2 s wall, 850 MB peak RSS. No `/tmp/w11-ge0b-*` and no win11-files-i18n/archive/trash temp directories left. |
| en render per stem | – | `files`: `files::tests::file_texts_follow_the_settings_language_per_call_through_files_call` (`Files::call`), extended for m4. `files_launch`: `files::actions::tests::launch_errors_render_in_the_context_language_with_their_keys`. Both ok. |
| No frozen-language text | `grep -nE 'static \|OnceLock\|LazyLock\|lazy\|thread_local\|cache' host/src/files/*.rs` | The same hits as §6. The fix round added none: `load_error` renders per call, and `LIBRARY` still caches only the `LoadError` kind. |
| Status | `git status --short` | only my files: the 13 host/src/files/*.rs, host/i18n/files.tsv, files_launch.tsv, tests/i18n/{BASELINE-CHANGES.rust-r2c, allow-host-r2c.txt}, notes/claude/round2/ (this note, r2c-review.md, r2c-plans/). The build and test logs are in `.rt/` (git-ignored, like the review's). |

### Left, and decisions for the coordinator (unchanged from §8 except item 1)

1. Merge `allow-host-r2c.txt` (4 rules) and `BASELINE-CHANGES.rust-r2c` (1 D-G1a line, 9 technical lines).
2. `[space]` warnings: keep them (my default), or add a piece marker to check_rust.
3. "(s)" for n ≠ 1 is still held back as cosmetic (D-G1b if Robin wants it).
4. For R2-D: the corrected `clean_uris(…, lang)` hand-off (§8 item 1), and the exact duplicates `nlNames` (57) / `twins`
   in `apps/catalog-data.json`.
5. Not run: the GNOME harness (not asked).
