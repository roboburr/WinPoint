# R2-C review: files/** conversion (round 2, 2026-11-05)

Adversarial review of step R2-C in W = `.wt/r2-c` (branch `r2/c`, HEAD d666df54, nothing committed). I edited no
product file. Scratch scripts and logs are in `.rt/review-r2c/` (`render.py`, `sites.py`, `check.log`,
`test.log`). Heavy runs: `r2c-rv-check` and `r2c-rv-test`, both through `tools/heavy.sh` (`tests/logs/heavy-events.log`).
No win11-* binary, gnome-shell, gsettings or dconf ran.

**Verdict: pass.** There is no blocker and no major. Dutch is byte-identical at every converted site, except the
accepted D-G1a `.one` row. Every check of the done-when list passes when I run it myself. The 4 minors are English
wording and narrow test gaps. The 6 notes include 2 small inaccuracies in the step's note and one correction to
the R2-D hand-off.

## 1. Checks I ran (from W's root, final state)

| Check | Result |
|---|---|
| `drift_rust.py --paths host/src/files/*.rs --warnings` | 13450 frozen, 1243 present, 239 mapped or listed, 143 rows, 162 new; **0 errors, 0 warnings** |
| `drift_rust.py --warnings` (with the ratchet) | 13211 present, 239 mapped, 334 new (bilingual-pair 6, log 4, technical 320, user-visible-dutch 3, user-visible-english 1: all GE1b's, none in files/), 178 keys, 20 allow-listed; **0 errors, 0 warnings**. `--new` lists no files/ literal |
| `extract_rs.py list` on all 13 owned files | **0 sites** in each file. `--all`: the 162 new literals of files/ are keys, json param names (`name`, `item`, `path`, `ext`), the second `"untrusted"` code, and GE1b's `key`/`params` |
| `check_rust.py --warnings` (node on PATH) | 24 stems, 143 rows; **0 errors, 4 warnings**: all 4 are `[space]` on `files.allOfTypePrefix` and `files.libarchiveLacks` (piece rows, nl and en) |
| `check.py` | **0 errors**, 56 warnings (glossary 52, all UI; rust 4) |
| `heavy.sh --tag r2c-rv-check -- cargo check --offline --locked -j2 --all-targets` | rc 0. Warnings: lib 28, lib test 26, win11-launch 2 (baseline). In files/: `mod.rs:367` native_watch, `trash.rs:230` list, `dry.rs:106` PermissionsExt (test). All 3 exist at d666df54: I checked trash `list` (no caller there) and the dry test import (unchanged in the diff) |
| `heavy.sh --min-mb 2048 --tag r2c-rv-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | rc 0. lib **273 passed, 8 ignored**; win11-launch 5 passed, 1 ignored. **16/16 oracle tests** ok (shell `oracle_tests::*` 8, disks python cases 7, `system::reference_contracts` 1). `every_converted_site_renders_its_3b898e6_dutch` ok, both new en tests ok, the 3 updated tests ok. `logs/guard-calls.log` 0 bytes; no `/tmp/w11-ge0b-*` left |
| Independent nl/en render (`.rt/review-r2c/render.py`) | Reads each `#@` literal from `git show 3b898e6:<path>` with its own Rust string reader and re-checks sha256. Renders the 3b898e6 format string and the row with the same arguments, incl. a name containing `{item}` and ‘ ’. **170/170 byte-identical** (150 nl + 20 en halves) |
| Call-site key choice (`.rt/review-r2c/sites.py`) | Compares, per fn and in order, the 3b898e6 user-visible literals with the keys now at the call sites. All 173 old literals are accounted for. Differences are only placeholder renames, the hoisted copy word and into-itself marker, the `quotedName` piece, `LoadError::message`, and the 4 allow-listed literals |
| Error codes | The multiset of codes per file is unchanged. The only extra entry is a second `"untrusted"` (one `ApiError::tr` per branch) |
| Keys | Every `files.*`/`files_launch.*` key used in code exists. No row is unused. The other `files.*` literals are method or event names |
| Frozen ApiError block | md5 of the block is identical to d666df54 |
| rustfmt | `rustfmt --edition 2024 --check` is clean on scratch copies of d666df54 and of the current files. So the formatting run added no unrelated churn |
| Ownership | `git status --short` lists only the 13 files/*.rs, the 2 stems, the 2 tests/i18n files and notes/claude/round2/. `git diff --name-only d666df54` is all inside host/src/files and the 2 stems. `types.json`, `launcher-names.json` and the oracles are untouched |

## 2. What I verified as correct (claims that hold)

* **Dutch.** Every converted site renders 3b898e6's bytes. Composed sentences are identical too:
  * `{item}` = `quotedName`/`thisItem` inside the `io_error` sentences;
  * prefix + `type_name`;
  * `libarchiveLacks` + symbol.
* **BASELINE-CHANGES.rust-r2c.** Every line is allowed:
  * The `key:files.emptyTrashFailed.one` line is D-G1a. Its nl is byte-equal to H's `i18n/host/files.tsv:92`.
  * The 9 removed literals are all technical in `rust-frozen-all.tsv`: "nl" 6, "en" 1, "empty" 2.
* **Language source.** In services, every site uses `ctx.lang()` or a `lang` passed down from it.
  * `current_language()` remains only in 3 places: `abs()` (used by apps), `open_external_uri` (window.rs ignores the result) and `Clipboard::new` (start-up error, stderr).
  * `event_snapshot` uses `Lang::Nl`, but drops its errors.
* **No frozen text.**
  * `archive_ffi::LIBRARY` now holds a `LoadError` kind, not text.
  * `Reader`/`ArchiveWriter` hold a per-call `Lang`.
  * `COMMENTS` is keyed by (mime, lang).
  * `TYPES`/`NAMES` hold both languages.
  * The trash and icon caches hold no text.
  * Nothing stores text to show later: failed[] texts are shown at once.
* **Extra 1.** `Files::context()` costs one `language_in` stat. `show_hidden` is lazy and equals 3b898e6 for a missing, corrupt or object-form file. The external-backend path is unchanged.
  * I tried to break it with a switch mid-cache: `language_in`'s inode/ctime stamp plus the 50 ms RACY rule re-read the file correctly.
  * The UI passes `showHidden` in every folder `files.list` call except `explorer/main.js:1056`. The desktop's trash listing goes to trash.rs, which does not use it. So the lazy full read is rare.
* **Extra 2.** The 3 tests now expect English. I grepped every remaining Dutch assert in the files tests: each one runs in an nl context.
* **Extra 3.** Pieces are one row each, and no merge was made.
* **Extra 4.** The XDG fallback names in `places.rs` are unchanged.
* **Extra 5.** All 47 entries of `types.json` have [nl, en]. The `launcher-names.json` reasoning holds. The duplicate claim is exact: `catalog-data.json` `nlNames` (57) equals `NL_NAMES`, and `twins` equals `SAME_NAME_AS_OURS`.
* **UI matching.** No vendor/ui code matches `files.*` keys or message text.
  * `bridge.js:280` matches `code === 'untrusted'` + `data.canTrust`/`data.path`. Code and data are unchanged.
  * “Allow launching” is `launchAllow` (bridge.js:44), and the UI uses “{name}” quotes in en, the same as `files.quotedName`.
* **Allow-list.** All 4 rules are narrow (exact text or fn+sha).
  * The 3 start-up errors reach only stderr: `Runtime::new` from lib.rs and from portal.rs `setup?`.
  * The `.trashinfo` rule covers a file format (C9).

## 3. Findings

| # | Sev | Finding | Evidence | Fix |
|---|---|---|---|---|
| m1 | minor | The libarchive load error is the one converted site whose envelope has no `key`/`params` (C2). Its LoadError rendering is also untested. | `archives.rs:82-84`: `ffi::library().map_err(\|error\| ApiError::new("unsupported_archive", &error.message(lang)))`. Every other converted site uses `ApiError::tr`. | For `LoadError::Missing`, use `ApiError::tr("unsupported_archive", lang, "files.libarchiveMissing", NO_PARAMS)`. For `Symbol`, use `ApiError::tr(…, "files.libarchiveLacks", NO_PARAMS)` and then set `message = error.message(lang)` (key names the piece). Add a test that `LoadError::Symbol("x").message(Lang::En) == "The archive library is missing x"`, and the nl equivalent. |
| m2 | minor | Some en rows keep Dutch sentence structure or calques (the gate's "natural phrasing"). Most came from H, offered by the converter, so they are allowed, but they read badly. | `files_launch.untrusted` "…isn’t trusted (yet), so starting it is blocked … Do you trust where it came from? Then choose “Allow launching”." ("Vertrouw je de bron? Kies dan …"). `files_launch.untrustedDrive` "…shortcuts on this drive aren’t started." `files.failedFor` "That didn’t work for {item}." `files.unknownKind` "Unknown kind of file." `files.badBudget` "Invalid search time." | Run `extract_rs.py en <key> "…" --apply`, e.g. untrusted: "This shortcut isn’t trusted yet, so it was blocked for your security. If you trust its source, choose “Allow launching”." (keep the label exactly). untrustedDrive: "For your security, shortcuts on this drive can’t be started." failedFor: "Something went wrong with {item}." unknownKind: "Unknown file type." badBudget: "Invalid search time limit." |
| m3 | minor | nl "schijf" is "disk" in 2 rows and "drive" in 6, often in the same flow (a full drive vs a read-only drive). | "disk": `files.noSpace` "There isn’t enough space on the disk.", `files.noSpaceExtract`. "drive": `files.readOnly`, `nameNotOnDrive`, `noTrash`, `otherDrive`, `files_launch.untrustedDrive`, `untrustedDriveTrust`. | Use "drive" (File Explorer's word): "There isn’t enough space on this drive." and "…on this drive to extract this archive." |
| m4 | minor | Narrow en test gaps: some English paths are never asserted. | Not covered: `Files::call`'s early errors through the new `Files::lang()` (`offline`, `bad_args` for a non-object), `LoadError::message`, the restore/transfer `failed[].error` texts, and `multi_properties` "All of type "/"Multiple types". The files test covers rename/list/createFolder/copy/properties/places. | Add these to `file_texts_follow_the_settings_language_per_call_through_files_call`, e.g. `service.call("files.rename", &json!([]))` → "Invalid input." in en. |
| n1 | note | Two count slips in the step's note (`r2c.md`). | §1 says "116 rows: 93 in `files`, 23 in `files_launch`. One more row is new Dutch text". But files.tsv has 93 rows *including* `.one`: 115 mapped + 1 new = 116, and check_rust's 143 = 116 + core 14 + window 13. §2 says `ctx.language == "nl"` ×7, but BASELINE-CHANGES has 6 "nl" lines (+1 "en", +2 "empty" = 9). | Correct the note. |
| n2 | note | The R2-D hand-off "use `files::abs_in(&json!(uri), ctx.lang())` at apps/mod.rs:160, where `ctx = files.context()`" is not literal: `:160` is inside `clean_uris(value)`, which has no ctx. | `apps/mod.rs:143` `fn clean_uris(value: &Value)`; caller `:112` has `ctx`. | Tell R2-D to add a `lang` parameter: `clean_uris(&args["uris"], ctx.lang())`. Today the effect is nil, because `current_language()` reads the same settings file as `files.config`. |
| n3 | note | `Files::lang()` runs while `jobs` is locked (the shutting-down branch). With `WIN11_REFERENCE_INSTANCE` it does an HTTP `settings.get` under that lock. | `mod.rs:283-292` | This affects tests only. Compute the lang before the lock if you touch it again. |
| n4 | note | `upper_first` (en) changes `message`, but `params.item` stays "this item". So key + params do not reproduce the en message exactly for a nameless path. | `mod.rs:812-814` | This is harmless: the UI shows `message` (C2), and the path is practically unreachable. Keep it. |
| n5 | note | The 4 `[space]` warnings are real piece rows (extra 3). | check_rust output | Keep them (I agree with the step's default), or let check_rust accept a `piece` marker. |
| n6 | note | `places.rs` keeps `i18n::pick(lang, "Downloads", "Downloads")`, a pair whose two halves are the same word. It exists only so that the literal multiset (drift) stays unchanged. | `places.rs` "downloads" row | It is harmless and commented. A plain "Downloads" would need 2 technical BASELINE-CHANGES lines. Keep it. |

## 4. For the coordinator

* Merge `allow-host-r2c.txt` (4 rules) and `BASELINE-CHANGES.rust-r2c` (1 D-G1a line, 9 technical lines). All are true.
* m1-m4 can be done in a short fix round with `extract_rs.py en` plus 2-3 test asserts. None changes Dutch.
* R2-D: see n2. The `nlNames`/`twins` duplicates are confirmed exact.
