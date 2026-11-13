# R2-D: apps and native glue (round 2, 2026-11-05)

W = `.wt/r2-d` (branch `r2/d`, from d666df54; `git status` was empty and HEAD d666df54 at the start). Nothing
committed. No win11-* binary, gnome-shell, installer, gsettings/dconf ran outside the sandboxes (`run-tests.sh`, the
GNOME harness). Every cargo and harness run went through `tools/heavy.sh` (tags `r2d-*`). G's main tree is untouched
(`git -C G status --short` empty; G's `tests/build/r1-debug` still passes its SHA256SUMS).

## 1. Result

* **81 Dutch call sites converted** in 9 files into 69 rows of 5 stems (`apps` 17, `picker` 13, `transfers` 12,
  `launch` 17, `gnome` 10), plus the English halves of 2 inline pairs: 83 `#@` mappings. Dutch is byte-identical
  (`every_converted_site_renders_its_3b898e6_dutch` ok). Every en row is en-GB with ’ (D-G23). Corrected in the fix
  round (review n4): 12 apps rows were H's en with ’ instead of ', taken where the converter offered H's exact-nl
  row (allowed); after the fix round's m3/m4 changes 10 still are, the other 59 rows are new English.
* **No Dutch text change.** `BASELINE-CHANGES.rust-r2d` has 62 lines, all without a visible change: 58 JSON values
  and 2 JSON keys of the launcher-names dedupe, and 2 technical `"en"` literals of the rewritten inline pairs.
* **Extras:** F38 fixed (English app names on a Dutch system) with fixture tests; Start tiles refresh at once (file
  monitor + rebuild before `apps.changed`), measured in the harness; launcher-names table deduplicated; picker WM class
  fixed (GE1b o2) with harness evidence; `mod i18n` in bin/launch.rs; "Alles" unchanged.
* **Checks:** drift 0/0 (paths and whole tree), check_rust 0/0, check.py 0 errors, cargo check without a new warning,
  `run-tests.sh` green (lib 281 = 271 + 10 new, 8 ignored; 16/16 oracles; guard log empty), GNOME scenarios 61/61,
  16/16, 10/10.
* **Blocker worked around:** the GNOME harness cannot run from a worktree (its private system-bus socket path is 102
  bytes, dbus-daemon allows 99). Minimal patch in §8; the runs used a byte-identical copy (§6).

## 2. What I converted, per file

The plans fed to `extract_rs.py convert --plan -` are in `notes/claude/round2/r2d-plans.md`. Before converting I gave
the pure functions a language parameter (no literal touched), so each site uses `@lang` and an en test can reach the
real call site. Production passes the same language as before: Win11's settings file (C1).

| File | Sites → rows | Language | By hand | Not converted (allow-listed, §4) |
|---|---|---|---|---|
| apps/mod.rs | 7 → 4 (`shuttingDown`, `invalidInput` ×4, `invalidApp`, `linkCantOpen`) | `i18n::language_in(&self.config)` once per `apps.*` call (one cached stat; replaces `files.context().language`, a full settings read, for every method except launch/openUri, which still build their `files::Context`) | – | `Appbewaking starten: {e}` (start-up) |
| apps/pertype.rs | 8 + 1 en half → 7 (`invalidFileType`, `schemeLinks`, `invalidInput`, `invalidSearch`, `invalidApp`, `appNotFound`, `defaultNotChanged` ×2) | `lang: &str` (now also passed to `selected`, `integer`, `set`) | pair `if lang=="en" {"{s} links"} else {"{s}-koppelingen"}` → one `trf(apps.schemeLinks)`; `map --en --spec {s}=scheme` | – |
| apps/defaults.rs | 11 + 1 en half → 9 (`invalidPath`, `fileGone`, `explorerName`, `unknownDefaultKind`, `invalidApp`, `cantBeDefault`, `somethingWentWrong`, `explorerNotInstalled`, `appNotFound`, `defaultNotChanged` ×2) | `lang` (also into `selected_default`, `internal`, `set_default`) | pair `"File Explorer"`/`"Verkenner"` in `json!` → `tr(apps.explorerName)`; `map --en` | 2 io::Error texts nobody shows |
| apps/recent.rs | 1 → 1 (`historyNotCleared`) | `Lang::of_settings(settings)`: the settings document `apps.clearRecent` already reads | – | – |
| apps/catalog.rs | 0 texts | – | F38, dedupe (§5) | – |
| picker.rs | 14 → 13 (`invalidOption` ×2) | `current_language()` in `create`/`submit` (the GTK process's file), passed into `selection`/`validate` | `map_err(\|_\| &tr(…))` → `tr(…)` (1) | – |
| transfers.rs | 12 → 12 | `call()` keeps its signature and passes `current_language()` to the new `call_in` (runtime.rs is frozen) | – | `Tab-overdrachtsmap moet privé zijn` (start-up) |
| process_scope.rs | 12 → 11 (`nonUtf8Argument` ×2) | `current_language()` in `start`/`spawn`/`spawn_pid_with_argv0`, passed down | 3 `map_err(\|_\| &tr(…))`, 1 `ok_or(tr().into())` → `ok_or_else` | `ensure_slice` (start-up) |
| gnome.rs | 10 → 10 (incl. `:25`, `gnome.actionFailed`) | `current_language()` per bridge call / region update (one stat), passed to `call_raw`, `decode_reply`, `rectangles` | – | – |
| launch.rs | 3 → 3 | `command()` keeps its signature (shell.rs, files/actions.rs call it) and passes `current_language()` to `command_in` | – | – |
| bin/launch.rs | 3 → 3 (`appNotFound`, `missingUri`, `unknownRequest`) | `i18n::current_language()` in `run()`, passed to `launch()`; the win11-launch process inherits WIN11_GNOME_CONFIG_DIR/XDG_CONFIG_HOME (launch_context_for keeps them) | `mod i18n` (§5) | 2 CLI usage errors (stderr only) |
| wallpaper.rs | 0 | – | – | all 7: their only consumer is settings_runtime.rs's `eprintln!` |
| portal.rs, portal/** | 0 | – | – | English diagnostics for stderr / D-Bus (13 literals) |
| launch_context.rs, launch_context/portal.rs, recent_datetime.rs | 0 user texts | – | – | – |
| backend.rs | not touched (optional; gate §3: the internal reference backend stays) | | | |

Refused by the converter: none of the planned sites. The converter's `bare literal (review)` output was fixed by hand
in 4 `map_err` closures (a reference to a temporary) and left as is in `gnome.rs`'s two `.unwrap_or(&tr(…))` (the
temporary lives to the end of the statement; rendered only on that error path) and the two `json!` pairs (rewritten).

Wording choices (en-GB, Windows 11): "Verkenner" = File Explorer, "standaard-app" = default app, "Er ging iets mis.
Probeer het opnieuw." = "Something went wrong. Try again." (glossary), "bestandskiezer" = file picker, "map" = folder,
"koppeling" (GNOME-koppeling) = link, as the extension's "notification link". Texts without a full stop in nl have none
in en. Branding Win11Shell kept (D-G16). check_rust reports no glossary, label or en-dutch warning.

## 3. Dutch changes

None that a user sees. `tests/i18n/BASELINE-CHANGES.rust-r2d` (62 lines):
* 57 `apps/catalog-data.json` `nlNames` values and 1 `twins` nl value: the copy of `files/launcher-names.json`'s
  `NL_NAMES` / `SAME_NAME_AS_OURS` is gone (§5.3); the names are read from that file, byte-identical.
* 2 `apps/catalog.rs` JSON keys (`"nlNames"`, `"twins"`).
* 2 technical `"en"` literals (defaults.rs#74, pertype.rs#14): the `lang == "en"` tests of the two inline pairs.

No D-G1a plural applies to R2-D's texts (no count), and no D-G2 text is R2-D's.

## 4. Allow-list rules (`tests/i18n/allow-host-r2d.txt`)

Rule used (as GE1b did for lib.rs/runtime.rs): a text that only stderr or a D-Bus peer receives stays (D-G15); every
text that can reach a page or another app's dialog is converted.

| Rule | Reason |
|---|---|
| apps/mod.rs `text:Appbewaking starten: {e}` | start-up error: Monitor::new → Apps::new → Runtime::new → lib.rs run → eprintln |
| transfers.rs `text:Tab-overdrachtsmap moet privé zijn` | start-up error: Transfers::new → Runtime::new → run → eprintln |
| process_scope.rs `fn:ensure_slice` (3 texts) | only for Role::Shell, i.e. from ensure_native (lib.rs run / portal::run → eprintln) |
| apps/defaults.rs `text:Het standaard-app-slot moet privé zijn.`, `text:Een app-id bevat een nulbyte.` | io::Errors that every caller replaces (`map_err(\|_\| …)`, `.unwrap_or(false)`) |
| wallpaper.rs `class:user-visible-dutch` (7 texts) | `wallpaper::sync`'s errors reach only `eprintln!("GNOME-achtergrond bijwerken: {error}")` (settings_runtime.rs) |
| bin/launch.rs `Launch-opdracht ontbreekt`, `Te veel argumenten` | CLI usage of win11-launch: no ack socket yet, so only main's eprintln prints them |
| portal.rs / portal/external.rs `class:user-visible-english`, portal/model.rs `fn:bad` | the FileChooser portal's English diagnostics: stderr, or D-Bus errors to xdg-desktop-portal; the parent window titled "Win11 FileChooser parent" is never mapped. The pickers it opens are picker.rs (converted) |

So the `wallpaper` stem stays header-only, and has no en render test.

## 5. Extras

1. **F38** (`apps/catalog.rs` `build`): `ensure()` passes a translator exactly when Win11's language differs from the
   system's. In English with a translator (a non-English system), the name, generic name and comment now come from
   the entry itself (new `english()`; order as corrected in the fix round, review m5/n4: `key[en_GB]`, then
   `key[en]` or the `en` gettext translation, then the untranslated `key=`, each only when not empty), GIO's
   system-language name only when the entry has no English name, and no generic name or comment rather than GIO's
   system-language one. A hidden entry (a default
   handler in Open with / Default Apps) gets the same English name. GIO's Dutch name stays a search keyword. Dutch,
   and English on an English system, take exactly the old code path. Tests (fixture .desktop files, both system
   locales): `english_on_a_dutch_system_takes_each_entrys_english_texts_not_gios_dutch_ones` (Files, Colour Picker
   from Name[en_GB], hidden "Link Handler", no-file entry keeps GIO's name) and
   `dutch_and_same_language_catalogues_keep_todays_names` (nl on en, nl on nl, en on en).
2. **Start tiles after a fast switch** (`apps/mod.rs` Monitor): the 150 ms poll reads the language with
   `i18n::language_in(&config)` (one stat instead of `files.context()`); a `GFileMonitor` on the config directory
   (WATCH_MOVES; the store writes by rename) checks at once when settings.json changes, the poll stays as fallback; on
   a change the catalogue is rebuilt on its own thread before `apps.changed` goes out, so the page's `apps.list` is a
   cache hit in the new language. `Apps::call` also takes the language by one stat. Harness (§6): Start opened 0.1 s
   after a rename is clean after 321-335 ms (nl) and 321-322 ms (en) in two runs, against 338-452 / 386-560 ms with the
   round-1 build and 336 / 326 ms with 3b898e6; all 61 checks pass in both runs (round 1 run 1 failed 2). **The floor
   that is left is the UI's** `bridge.on('apps.changed', debounce(load, 400))` (vendor/ui/js/shell/shell.js:159): 400 ms
   after the event minus the 100 ms open delay ≈ 300 ms; the host part is now about 20-35 ms (it was up to ~260 ms).
3. **launcher-names dedupe** (plan-critique H): `catalog-data.json`'s `nlNames` equalled `files/launcher-names.json`
   `NL_NAMES` exactly (57 ids, values and order) and `twins` equalled `SAME_NAME_AS_OURS`. catalog.rs now reads that
   file with `include_str!("../files/launcher-names.json")` (not edited; R2-C's) and the copy is removed from
   catalog-data.json. Test `gnome_names_come_from_verkenners_launcher_table`; the existing Nautilus / "Schijven (GNOME)"
   tests pass. 62 BASELINE lines (§3).
4. **Picker WM class (GE1b o2), fixed** (`picker.rs` configure): GTK3 creates the Wayland xdg_toplevel only at map and
   then gives it `g_get_prgname()` (the caller's app id); `gdk_wayland_window_set_application_id` before that does
   nothing, so the realize hook never worked. The call moved to the `map` signal (its handlers run after GtkWindow's map,
   when the toplevel exists, before the first buffer). Harness `tests/gnome/scenarios/r2d-picker.py`, one modal picker
   (as every in-app `shell.pickFile` caller) and one `{modal:false}`:
   * r1 build: both `io.github.win11shell.Settings` (o2 reproduced); r2d build: both `io.github.win11shell.FilePicker`.
   * Both builds: GTK app id null, transient for Settings, GNOME's WindowTracker gives Settings (grouping unchanged),
     both `shell.pickFile` calls settle as cancelled.
   * Look: the modal picker (MODAL_DIALOG) never gets the extension's own-window backdrop; its screenshots are equal
     apart from the file times in the list. The non-modal one (NORMAL) carried the Settings window's shadow entry with
     the r1 build and has none with the fix: frostedBridge.js's `windowIdentity()` keeps a FilePicker out of own-app
     effects on purpose. No Win11 UI caller opens a non-modal picker; only a file-portal caller asking `modal: false`.
     Decision 3.
5. **bin/launch.rs**: `#[allow(dead_code)] #[path = "../i18n.rs"] mod i18n;`. i18n.rs's 10 unit tests now also run in
   the win11-launch test target (5 → 16 passed: + 10 + my 1).
6. **"Alles"**: unchanged (`ownApps.alles.name` nl/en "Alles"); no R2-D row names it.

**en render test per stem**, each through the real call site, nl asserted too: `apps::tests::app_errors_render_in_the_settings_language_through_their_call_sites`
(pertype/defaults `call`, `clean_uris`; message and key), `apps::pertype::tests::a_link_type_without_a_description_is_named_in_the_settings_language`,
`recent.rs` backup-failure test (en message + key), `picker::tests::selection_errors_follow_the_settings_language`,
`transfers::tests::transfer_errors_follow_the_settings_language`, `gnome::tests::bridge_and_region_errors_follow_the_settings_language`,
`launch::tests::launch_errors_follow_the_settings_language`, `process_scope::manager_tests::a_non_utf8_environment_is_refused_in_the_settings_language`,
and in the bin `tests::launch_answers_follow_the_settings_language`.

**No frozen-language text:** grep of my files for `static`/`OnceLock`/`lazy`/`thread_local`/caches: pertype's
`kinds()` cache (holds `apps.schemeLinks` labels) is keyed by language; the catalogue state is keyed by language;
`data()`/`launcher_names()` hold both-language data, never rendered text; `own_icon`, recent's cache, picker's
`PENDING` and the AtomicU64 counters hold no text.

## 6. Verification (from W's root; final state)

| Check | Command | Result |
|---|---|---|
| Drift, my paths | `python3 -B tools/i18n/drift_rust.py --paths host/src/apps/{mod,catalog,defaults,pertype,recent,recent_datetime}.rs host/src/apps/catalog-data.json host/src/{picker,transfers,process_scope,gnome,wallpaper,launch,launch_context,portal}.rs host/src/bin/launch.rs --warnings` | 1,041 present, 205 mapped or listed, 96 rows, 100 new literals; **0 errors, 0 warnings** |
| Drift, whole tree | `python3 -B tools/i18n/drift_rust.py --warnings --summary` | 13,450 frozen; 13,245 present, 205 mapped or listed; 96 rows; ratchet 274 new (255 technical, 10 pair, 5 log, 3 user-visible-dutch, 1 user-visible-english; 110 catalogue keys, 21 allow-listed); **0 errors, 0 warnings** |
| My new literals | `drift_rust.py --new --paths <my files>` | 100 new: 95 technical, 1 log (the `.expect` text of `launcher_names()`), 4 classed bilingual-pair: the desktop-entry keys `"Name"` ×2, `"GenericName"`, `"Comment"` passed to `english()` (en/none, no warning). No `new:` warning |
| Catalogues | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` | 24 stems, 96 rows; **0 errors, 0 warnings** (labels checked with node) |
| All layers | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check.py` | **0 errors**, 52 glossary warnings, none in my files (UI and services/disks data.json) |
| Build check | `tools/heavy.sh --tag r2d-check -- cargo check --offline --locked -j2 --all-targets` | 0 errors; lib 28, lib test 26, win11-launch 2, win11-launch test 1 = the baseline. Warnings in my files are all pre-existing: pertype.rs:10 unused `HashMap`, picker.rs:35 `show_portal`, launch_context.rs:95/:114 in the bin |
| Tests | `tools/heavy.sh --min-mb 2048 --tag r2d-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | rc 0. lib **281 passed** (271 + 10 new), 8 ignored; win11-launch 16 passed, 1 ignored; other bins 0. Oracles **16/16** (shell `oracle_tests` 8, disks python cases 7, `reference_contracts` 1). `every_converted_site_renders_its_3b898e6_dutch` ok (lib and bin). `logs/guard-calls.log` 0 bytes. 15.1 s wall, 917 MiB peak RSS |
| Binaries | `tools/heavy.sh --tag r2d-build -- bash -c 'cargo build … && cp $(ls target/debug/win11-* \| grep -v "\.d$") tests/build/r2d-debug/ && sha256sum …'` | 12 executables + SHA256SUMS (12/12 OK, `cmp` = target/debug; the task's `cp target/debug/win11-*` would also copy the 12 `.d` files, so I left those out); `strings win11-app` has the new rows |
| Fast switch | `tools/heavy.sh --min-mb 2560 --tag r2d-switch -- python3 -B t/g/harness.py launch run --lang nl --build-dir tests/build/r2d-debug --source worktree --scenario tests/gnome/scenarios/ge1a-switch.py --out t/g/out/ge1a-switch.r2d` (and `-2`) | 61/61 twice (188.3 s, 184.3 s). `switch.fastStart`: nl lastDirty 321 / 335 ms (clean from 341 / 350), en 322 / 321 ms (clean from 335 / 332). Round 1 (cross-r1.reruns, r1 build): nl 452 / 338, en 560 (never clean, 2 checks failed) / 386; 3b898e6 (ge1a-switch.out): nl 336, en 326. Peak PSS 1,535 / 1,554 MiB, min MemAvailable 8,310 / 8,342 MiB |
| Start switch | same with `ge1a-start-switch.py` → `ge1a-start-switch.r2d` | 16/16 (as round 1), 115.1 s |
| Picker o2 | `r2d-picker.py` with `tests/build/r2d-debug` → `r2d-picker.out/r2d`; reference with G's r1 build → `r2d-picker.out/r1` | r2d 10/10; r1 8/10, the 2 failures are the known o2 WM class. Install 0 paths outside the run dir, no leftover process, no `i18n:` miss or panic in any app log of the 5 runs |

**The harness workaround.** From W the harness stops at "private system bus reported ''": dbus-daemon's
`_DBUS_MAX_SUN_PATH_LENGTH` is 99 and `W/tests/gnome/run/rt/system-bus` is 102 bytes (93 in G). The harness
derives everything from its own path (`HERE = Path(__file__).resolve().parent`, `G = HERE.parent.parent`), and a
symlink, an outer bwrap (nested user namespaces are denied here by AppArmor) or a run from G (it writes `run/` and
`--out` into G's tree) were no option. So I ran a byte-identical copy of `tests/gnome/harness.py` and
`tests/gnome/lib/` from `W/t/g/` (`cmp`/`diff -r` checked): its G is still W, `--source worktree` installs W's tree and
the socket path is 94 bytes. Outputs were copied to `tests/gnome/scenarios/{ge1a-switch.r2d,ge1a-switch.r2d-2,ge1a-start-switch.r2d,r2d-picker.out}/`
(their results.json name `t/g/...` paths); `W/t/` and the first attempt's `tests/gnome/run/` are deleted. For the r1
reference the harness needs the build inside its G, so `tests/build/ref-r1-debug` was a hard-link copy of G's
`tests/build/r1-debug` (read only; removed; G's SHA256SUMS still pass).

## 7. Deviations from the gate and contracts

1. Harness run from a copy (above); minimal tool patch in §8.
2. Output directories `tests/gnome/scenarios/ge1a-*.r2d*` as the task names them (not `r2d-*`).
3. Plan files kept as `notes/claude/round2/r2d-plans.md` (the gate says "your notes directory"; my pattern is `r2d*.md`).
4. Texts that only reach stderr / D-Bus are allow-listed instead of converted (§4); the `wallpaper` stem stays empty.
5. picker.rs changed beyond texts (o2), and apps/mod.rs's catalogue monitor (extra 2): both were asked for.
6. The build snapshot leaves out the `.d` files the task's `cp target/debug/win11-*` would copy.

## 8. Left, and decisions for the coordinator

1. **Harness in worktrees (tool fix, frozen file):** shorten the private system-bus socket name, e.g. in
   `tests/gnome/harness.py` `'system-bus'` → `'sbus'` at :125, :905, :917 and :1387 (RT/sbus is 96 bytes from a
   `.wt/r2-x` worktree; the session bus RT/bus is 95, the limit 99). Until then, R2-B's `ge1b-perf` run hits the same
   failure from its worktree.
2. **Allow-list vs convert** for stderr-only texts (§4): accept (my default), or convert them (wallpaper's 7, 6
   start-up/CLI texts) — about 13 more rows, no user would see them.
3. **o2 side effect:** keep the FilePicker identity (my default; in-app pickers look the same), and if a non-modal
   portal picker should keep a shadow, give FilePicker windows their own shadow in frostedBridge.js in round 3
   (extension frozen now).
4. **Round-3 UI item:** Start's tile names wait for `debounce(load, 400)` on `apps.changed` (shell.js:159); loading
   at once on a language change would remove the last ~300 ms (ge1a-switch's fast-start check allows 1 s).
5. The 62 dedupe/technical BASELINE lines are no Dutch change; please show them under "no visible change" in the report.
6. Not done / not verified:
   * F38 on a real Dutch-locale machine (the NucBox); fixture tests only.
   * `Catalog::brief_id`'s fallback for an id outside `AppInfo::all()` still uses GIO's name (rare).
   * pertype's last-resort `gio::content_type_get_description` stays in the system locale (D-G11).
   * backend.rs (18 Dutch texts, the internal reference backend) untouched, as gate §3 lists it.
   * `apps.launch`/`apps.openUri` still build `files::Context` (R2-C replaces its full settings read).
7. **Files** (`git status --short`): the 5 stem files, 12 host/src files incl. catalog-data.json, `tests/i18n/allow-host-r2d.txt`,
   `tests/i18n/BASELINE-CHANGES.rust-r2d`, `tests/gnome/scenarios/r2d-picker.py`, the 4 output directories, and
   these two notes. `tests/build/r2d-debug/` (230 MB) is git-excluded.

## 9. Fix round (after `r2d-review.md`, verdict pass, 6 minor + 5 notes)

**Start state.** `git -C W status --short` and `git -C W diff --stat` matched the review's view (17 modified files:
the 5 stems and 12 host/src files incl. catalog-data.json; untracked: the notes, the allow and BASELINE files, the
picker scenario and the 4 output directories). Nothing in my allowlist had changed beyond what the step and the
review describe; the frozen files are unchanged against d666df54. Nothing committed.

### 9.1 Per finding

| Id | What I did |
|---|---|
| m1 (test hang, run-tests.sh) | No R2-D change (the review's own conclusion: the futex wait is in the files/read.rs `icon_catalog` → `current_theme()` path, which builds a `gio::Settings` per call; not my file). My test run was guarded without touching the frozen runner: `tools/heavy.sh … -- timeout -k 10 900 tests/rust-sandbox/run-tests.sh --no-fail-fast` (not triggered: 14.6 s). Coordinator: the runner hang guard and the read.rs cache, as the review says. |
| m2 (allow rules too wide) | Fixed. No `class:` or file-wide `fn:` rule left: each rule names one text, with its fn where that is narrower (`fn:image&text:…`). wallpaper.rs 7 rules, portal.rs 4, portal/external.rs 6, portal/model.rs 1 (`fn:bad&text:Invalid FileChooser {what}`), and process_scope.rs `fn:ensure_slice` split into its 3 texts as well. The nested-picker reason is corrected: an Err ack to a page's `shell.pickFile`/`shell.setRegions` UiRequest inside the portal process, unreachable because the only page the portal loads (the picker) sends neither; it would reach that page, not stderr. One new rule: catalog.rs `fn:english&text:{key}[en_GB]`, the desktop-entry key name of the m5 fix, which the classifier calls Dutch ("en"). 28 rules in all; check_rust reports none that matches nothing. |
| m3 | Fixed with the converter: `apps.defaultNotChanged` en = “The default app couldn’t be changed.” (the UI's `saveFailed` for the same nl, defaultapps/main.js:35). `apps.somethingWentWrong` left as the review says (glossary “Try again”). |
| m4 | Fixed with the converter. `gnome.invalidReply` en = “Invalid reply from the GNOME connection”; its translator comment (a `#` line, no row) now says “GNOME-koppeling: the D-Bus connection to the extension”. `apps.explorerNotInstalled` en = “File Explorer isn’t in the app menu yet. Run install.sh once more, then try again.” rather than the review's “… again and then retry.”: it removes the double “again” and keeps the glossary's “try again” (Probeer het opnieuw → Try again). |
| m5 (F38 order) | Fixed in catalog.rs. `english(translator, file, key)`: `key[en_GB]`, then `Translator::get` in English (`key[en]` when it differs from `key=`, or the `en` gettext translation), then the untranslated `key=`; every empty value is skipped. In English with a translator the name falls back to GIO's name only when the entry has no English name; the generic name and the comment are `""` when the entry has no English text (instead of GIO's system-language text), as the Dutch branch already does. The hidden-entry branch uses the same helper. The Dutch branch makes the same `Translator::get` calls in the same order as at 3b898e6, now in one `if lang == "nl"` block; the hidden branch's English test is written `lang != "nl"` like the visible branch's `else` (this also keeps catalog.rs's frozen `"nl"` literal count, so no BASELINE line). New test `english_texts_follow_en_gb_then_en_then_the_untranslated_key_and_never_the_system_language` (fixtures on a Dutch system: Name[en_GB] beats Name[en]; an empty Name[en_GB] falls back to Name[en]; empty Name[en_GB] and Name[en] fall back to Name=; an empty GenericName[en_GB] falls back to GenericName=; GenericName/Comment only in [nl] give ""; and the Dutch names of the same entries in Dutch). I did not run it against the reviewed code; by the GLib behaviour the review probed it fails there on Paint's name, Notes' generic/comment and Clock's name. Note §5.1 corrected (n4). |
| m6 (picker taskbar identity) | Check added to `r2d-picker.py` and run: **the review's suspicion was a real regression.** With the first-pass build (`r2d-picker.out/r2d-old`), the non-modal picker got a running taskbar button of its own: windows.list gave appId `io.github.win11shell.FilePicker`, desktopId `io.github.win11shell.Settings.desktop` (WindowTracker), and shell.js `resolve()` mapped that through the alias `settings` to `org.gnome.Settings.desktop` (“GNOME-instellingen — 1 venster geopend”). The round-1 build (`r1`) groups it on the Settings button (“Instellingen — 2 vensters geopend”). **Fix (picker.rs):** the map hook now also gives the picker its caller's GTK application id with `gdk_wayland_window_set_dbus_properties_libgtk_only` (public `gdk/wayland/gdkwaylandwindow.h`; what GtkApplication sends for its own windows; the picker is detached from its GtkApplication, so GTK does not). That is the identity frostedBridge.js `windowIdentity()` describes (“retains its parent's GTK application ID but has its own Wayland protocol role”), and panelBridge `_windowJSON()` reads the GTK id first. New build (`r2d`): 16/16: both pickers WM class FilePicker and GTK app id Settings, no picker adds a button, the non-modal one is on the Settings button (2 windows), GNOME's WindowTracker still Settings, both calls settle as cancelled. What still differs from round 1: the non-modal picker has no extension shadow entry (r1: `shadow: true`), as the extension intends for a FilePicker. On the screenshot that is a band of about 8 px whose grey is 22 instead of 19-21 at the left edge (422 px differ in the 45 px margin; the modal picker: 0). Portal pickers (in the portal process, transient for the external app) are grouped by GNOME's WindowTracker as before, since the extension's map knows neither id. |
| n1 | Not changed: the callers (services/shell.rs, runtime.rs, notifications.rs, sysmon/actions.rs) are not R2-D's, and in production `i18n::config_dir` is the ServiceContext's directory. `launch::command_in` exists (private); gnome and process_scope already pass `lang` internally. A `pub(crate)` lang-taking entry point is a small change for the callers' owners when they convert. |
| n2 | Fixed (apps/mod.rs Monitor): `invalidate()` only when an app change is due; on a language change only `list(next)`, which `ensure()` rebuilds on a language mismatch and skips when a page's `apps.list` already rebuilt it; one `apps.changed` either way. ge1a-switch rerun 61/61, fast start nl 339 / en 320 ms (first pass 321-335 / 321-322 ms): no regression. |
| n3 | Fixed (gnome.rs `call_api`, `decode_reply`): `.map(str::to_owned).unwrap_or_else(\|\| tr(…))`, so the fallback is rendered only when the extension sent no message. |
| n4 | Note corrected: §1 (12 apps rows were H's en with ’, 10 after m3/m4; the other 59 rows are new) and §5.1 (lookup order). |
| n5 | The harness evidence of this round comes from a byte-identical copy again (W/t/g, deleted after the runs): sha256 harness.py `bea86ace76c107a8…`, lib/scenario.py `68fc151832ed680c…`, lib/__init__.py `ab8710330cc5e97f…`, the same as W's `tests/gnome/{harness.py,lib/}`, which are unchanged against d666df54 (`cmp`/`diff -r` before and after the runs). The socket patch is unchanged (§8.1). |

### 9.2 Done-when (from W's root, final state; no host/src file is newer than the build snapshot)

| Check | Command | Result |
|---|---|---|
| Drift, my paths | `python3 -B tools/i18n/drift_rust.py --paths host/src/apps/{mod,catalog,defaults,pertype,recent,recent_datetime}.rs host/src/apps/catalog-data.json host/src/{picker,transfers,process_scope,gnome,wallpaper,launch,launch_context,portal}.rs host/src/portal/{external,model}.rs host/src/bin/launch.rs --warnings` | 1,151 present, 205 mapped or listed, 96 rows, 100 new literals (95 technical, 3 bilingual-pair: the keys `"Name"`, `"GenericName"`, `"Comment"` passed to `english()`; 1 log: `launcher_names()`'s `.expect`; 1 user-visible-dutch: `{key}[en_GB]`, allow-listed); **0 errors, 0 warnings**, no `new:` warning |
| Drift, whole tree | `python3 -B tools/i18n/drift_rust.py --warnings` | 13,450 frozen, 13,245 present, 205 mapped or listed, 96 rows, ratchet 274 new; **0 errors, 0 warnings** |
| Catalogues | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` | 24 stems, 96 rows; **0 errors, 0 warnings** |
| All layers | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check.py` | **0 errors**, 52 glossary warnings, all in vendor/ui (none in my stems) |
| Build check | `tools/heavy.sh --tag r2d-check -- cargo check --offline --locked -j2 --all-targets` | rc 0; lib 28, lib test 26, win11-launch 2, win11-launch test 1 = the baseline. In my files only the pre-existing pertype.rs:10, picker.rs:35, launch_context.rs:114 (and the untouched portal/model.rs:25) |
| Tests | `tools/heavy.sh --min-mb 2048 --tag r2d-test -- timeout -k 10 900 tests/rust-sandbox/run-tests.sh --no-fail-fast` | rc 0, 14.6 s, 947 MiB peak RSS. lib **282 passed** (271 + 11 R2-D tests), 8 ignored; win11-launch 16 passed, 1 ignored. Oracles **16/16** (shell `oracle_tests` 8, the 7 disks python cases, `reference_contracts` 1). `every_converted_site_renders_its_3b898e6_dutch` ok (lib and bin). `tests/rust-sandbox/logs/guard-calls.log` 0 bytes |
| en render per stem | as §5 (unchanged); plus the new catalog test | ok |
| No frozen text | grep of my files for `static`/`OnceLock`/`lazy`/`thread_local`/caches around `tr(`/`trf(`/`trn(` | none new; the fix round adds no static or cache (§5's findings stand) |
| Binaries | `tools/heavy.sh --min-mb 2048 --tag r2d-build -- bash -c 'cargo build … && cp $(ls target/debug/win11-* \| grep -v "\.d$") tests/build/r2d-debug/ && sha256sum …'` | 12 executables + SHA256SUMS, 12/12 OK, `cmp` = target/debug |
| Picker (m6) | `tools/heavy.sh --min-mb 2560 --tag r2d-fix-picker[2] -- python3 -B t/g/harness.py launch run --lang nl --build-dir tests/build/<b> --source worktree --scenario tests/gnome/scenarios/r2d-picker.py --out t/g/out/r2d-picker.<n>`, b = the first-pass build (before the rebuild), a plain copy of G's r1-debug, the new build | `r2d-picker.out/r2d-old` 11/14 (the 2 taskbar checks of the non-modal picker and the summary fail: its own “GNOME-instellingen” button); `r1` 12/14 (the 2 known o2 WM-class checks; taskbar grouped); `r2d` **16/16**. 0 paths outside the run dir, no leftover process, no `i18n:` miss or panic in any app log |
| Fast switch (n2) | the same with `ge1a-switch.py` → `ge1a-switch.r2d-fix` | **61/61**, 197.3 s; fastStart nl lastDirty 339 ms (clean from 351), en 320 ms (clean from 340) |
| Start switch | the same with `ge1a-start-switch.py` → `ge1a-start-switch.r2d-fix` | **16/16**, 125.9 s |
| Ownership | `git -C W status --short` | only my files: the 5 stems, 12 host/src files, allow and BASELINE files, `r2d-picker.py` and its outputs, the ge1a-*.r2d* outputs, the notes. G's tree clean, G's r1-debug SHA256SUMS OK; `W/t/` and `tests/build/ref-r1-debug` removed |

Outputs: `r2d-picker.out/{r1,r2d}` replace the first pass's (they now carry the taskbar data); `r2d-picker.out/r2d-old`
is new (its full-screen `scenario-FAILED.png`, taken after the pickers had closed, is left out).
`ge1a-switch.r2d-fix/` and `ge1a-start-switch.r2d-fix/` hold `results.json` and `logs/` only (the 97 and 35
screenshots are left out; the first pass's directories have them).

**Files changed in the fix round:** host/src/apps/catalog.rs, host/src/apps/mod.rs, host/src/gnome.rs,
host/src/picker.rs, host/i18n/apps.tsv (2 en), host/i18n/gnome.tsv (1 en, 1 comment), tests/i18n/allow-host-r2d.txt,
tests/gnome/scenarios/r2d-picker.py and the output directories above, this note. `BASELINE-CHANGES.rust-r2d` is
unchanged (62 lines, none a visible Dutch change); no Dutch text changed.

### 9.3 Left, and decisions for the coordinator (replaces §8 items 3 and 6 where they differ)

1. Harness socket patch (§8.1) and run-tests.sh hang guard (m1): tool fixes for the coordinator. Rerun
   `r2d-picker`, `ge1a-switch` and `ge1a-start-switch` from W with the real harness once the patch is in.
2. Allow-list vs convert for stderr-only texts (§8.2): unchanged; the rules are now per text.
3. Picker identity: keep the FilePicker role plus the caller's GTK application id (taskbar grouping as round 1,
   now checked). Open for round 3 (extension frozen now): whether a non-modal FilePicker should get the own-window
   shadow in frostedBridge.js; only a `{modal:false}` caller sees it, and no Win11 page is one.
4. Round-3 UI: Start tiles load at once on a language change instead of after the 400 ms debounce (§8.4).
5. Not verified: F38 on a real Dutch-locale machine (fixtures only); m5's new test against the reviewed code.
6. Owner of files/read.rs: cache the theme before `icon_catalog` (m1).
