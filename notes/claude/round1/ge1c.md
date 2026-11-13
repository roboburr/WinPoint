# GE1c: English in the GNOME Shell extension and the installer (2026-11-05)

G = this repo (branch `claude/gnome-english-update`, HEAD b431c627 plus the round-1 working tree). Nothing was
committed, stashed or switched. Only GE1c's allowlist was edited; every "changed on disk" notice during the run was
my own edit (no foreign change in my files). Dutch stays default.

## 1. What changed

| File | Change |
|---|---|
| `extension/extension.js` | `LanguageWatch` (C1): reads only `GLib.get_user_config_dir()/win11-gnome/settings.json` (`"en"` exactly → en; missing file, missing key, unknown string, `"EN"`, object form, broken JSON → nl), `Gio.File.monitor_directory(WATCH_MOVES)` on that folder (rename = the store's write), and on the parent while the folder does not exist yet; 100 ms debounce; creates no files; cancelled in `disable()`. `TEXT` table (9 pairs). `_relabel()` on a switch: accessible names of Start/Search/Task view/system button, the search label (via `_layout`), `ctrlAltTabManager.removeGroup` + `addGroup` with the new name, the clock (`%d-%m-%Y` nl, `%d/%m/%Y` en-GB), a refresh of the app buttons. New: the bar also refreshes on `Shell.AppSystem` `installed-changed`, so pinned own apps follow win11-panel's C6 `Name=` rewrite. `Main.notify` texts bilingual. `readSettings()` (theme, taskbar, pins, alignment) is unchanged. |
| `extension/panelBridge.js` | `TEXT` (33 pairs): the 26 distinct `failure()` texts (29 calls) and the 7 launch-context app names (from `host/src/desktop-entries.json`, "Alles" in both, D-G5, instead of "Everything"). `_t()` renders in Win11's language at call time; codes unchanged; the language getter is passed to the three sub-bridges. |
| `extension/notificationsBridge.js` | `TEXT` (7 pairs): 4 failures + the test notification (host's `args.language` first, else the extension's language). C5: an unnamed GNOME source gives `appName ""` instead of English `"Notification"`. |
| `extension/desktopBridge.js`, `frostedBridge.js`, `shortcutBridge.js` | 1, 2 and 2 pairs; optional `language` constructor argument. |
| `scripts/install.py` | `--lang nl\|en` (also `--lang=X`, any case; bad value → both languages, exit 2), `-h/--help`, unknown option → exit 2. Language resolution (C1/C6/H D4/D13/F20): existing win11-gnome `settings.json` always wins → `--lang` → existing payload without settings = nl (D13, nothing stored) → H's `config/win11shell/settings.json` language (or nl if only H's payload dir exists) → one locale guess (H's `detect_info` rules ported: LC_ALL/LC_MESSAGES/LANG/LANGUAGE, D4 time zones and nl format locales, C → nl). A first install stores the choice as a COMPLETE `settings.json` (settings-defaults.json + language; 0600 in a 0700 folder, `.settings.lock`, rename; never over an existing file; GE0c F3). Messages: `--lang` if given, else that language; one language line on a first install, a note when `--lang` differs from the stored choice. `Name=`/`Comment=` from `host/src/desktop-entries.json` in the stored language (C6, F19). Build dir: `WIN11_BUILD_DIR`, else the first complete of `target/release`, `bin/`; every missing binary is named. Runtime preflight before any change: DT_NEEDED of all 12 binaries (own ELF reader, nothing executed) + `libarchive.so.13` (dlopen) resolved through `ldconfig -p` and the lib dirs, `python3-gi`, `glib-compile-schemas` → bilingual message with the Ubuntu packages; missing `update-desktop-database` → warning. Extension files = listing of `extension/` minus `schemas/` (C7, F18); stale top-level files removed. `T` table (33 pairs). `BINS = (…)` stays one line (the harness parses it). |
| `scripts/install.sh`, `package.sh` | pass `"$@"` through. |
| `scripts/uninstall.py` | `--lang`; messages in `--lang`, else Win11's language; removes every top-level file of its own extension folder; usage line names `--lang`. |
| `scripts/icon_bundle.py`, `file_portal.py` | `T` tables (10, 6 pairs), `LANGUAGE` set by install/uninstall. Logic unchanged. |
| `scripts/build.sh`, `package.py`, `verify-ui.py` | developer scripts: `--lang` or `WIN11_LANG` (default nl; they read no user settings). `verify-ui.py` logic unchanged. `package.py` also ships `tools/h-remnants.sh` (for the NucBox without a checkout). |
| `tools/h-remnants.sh` (new) | read-only inventory of H: `~/.local/bin/win11shell*`, `~/.local/share/win11shell`, `~/.config/win11shell` (settings language, installer state, hypr), `win11shell-*.desktop`, mimeapps defaults (`mimeapps.list`, `*-mimeapps.list`, legacy), GDM session entry and `/usr/local/bin/win11shell-session`, systemd target/drop-ins/wants, themes and icons from H's state file, GTK CSS blocks, running H processes. nl/en; exit 0 nothing, 1 found, 2 bad option. |
| `tools/i18n/check_pairs.py` (new) | checks every `// i18n: pairs` JS table, every Python `{nl,en}` dict (ast), `pair`/`section` lines in shell, and `desktop-entries.json`: both languages, non-empty, equal placeholders (`{x}`, Python format, printf), used keys exist, no one-language literal, no literal text left in `failure()`, `fail()`, `new Error()`, `Main.notify()`, `accessible_name`, `_button()`, `addGroup()`, `print()`, `SystemExit()`, `ValueError()`. |
| `tests/installer/` (new) | `run.sh` (heavy.sh + `tests/i18n/private.sh`), `sandbox.py` (bwrap: ro root, real HOME behind a tmpfs with only G bound back read-only, private /tmp /run/user /run/dbus, no network, own PID ns; private env; PATH stubs; strace), `test_installer.py`, `test_extension_text.py`, `ext/` (Node hooks + a fake GNOME Shell world + `dump.mjs`), `.gitignore` (`run/`). |
| `tests/i18n/BASELINE-CHANGES.ext` (new) | 38 `key:ext.*` rows (every nl change of the extension, tested exactly), 15 `key:install.*` rows, 5 behaviour rows. |
| `tests/gnome/scenarios/ge1c-extension.py` (new) + `.out/` | GE1h-API scenario for the review stage (120 KB results). |

## 2. Verification (commands from G, results)

* `python3 -B tools/i18n/check_pairs.py --warnings` → **0 errors, 0 warnings**; 143 pairs in 11 tables (6 JS, 6 Python, 5 shell files, 1 JSON).
* `node --experimental-default-type=module --check extension/*.js` → 6/6 ok.
* `tests/installer/run.sh` (heavy.sh `--min-mb 1024`, inside private.sh) → **35 tests OK in 227.8 s**:
  * **nl byte-identity with 3b898e6** (old `scripts/*.py` from `git show`, same case dir): Robin's update case (nl settings.json) → stdout, stderr and the whole installed tree identical; same on the **real tree** (vendor snapshot, the 12 `tests/build/3b898e6-debug` binaries, prefix path `ge1c sp %u $HOME `q` "dq"`); D13 update on an English locale identical; single missing binary message identical; `uninstall.py --files-only` output, tree and "Niets verwijderd" identical; `verify-ui.py` and `build.sh` nl output identical. nl first install: exactly one extra first line plus `config/win11-gnome/{settings.json,.settings.lock}`.
  * **strace**: real nl update 140,216 and real en first install 140,223 mutating syscall paths, **0 outside the case dir**; fast cases and uninstall also 0; no PATH stub was called. Negative control: the repo is read-only inside (EROFS), `~/.config/win11-gnome` is invisible, a write to the real home lands in the tmpfs and is flagged.
  * en: `--lang en` → 10 English `Name=` + English `Comment=`, English messages, 0 Dutch words; reinstall keeps en (settings.json byte-identical, no banner); `--lang nl` on it → Dutch note, stored en kept; switch to en then reinstall → English names (F19); locale guess rows (en_GB → en, en_US+Amsterdam → nl tz, LC_TIME nl → nl, C → nl, fr → en); 19 unit rows of the guess; H migration (en copied into a full doc, H file untouched, second install ignores H; no language → nl; payload only → nl; `--lang` beats H); 7 settings values (C1); bad/unknown options; folder symlink refused before any change; extension set = listing (extra module installed, removed again); build dir choice; preflight message nl/en with packages (libraries hidden); ELF reader = readelf; h-remnants (16 items found, 0 mutating syscalls, tree unchanged, en without Dutch; empty → exit 0).
  * Extension (Node, fake Shell): nl dump of `extension/` vs 3b898e6's differs in **exactly the 38 BASELINE-CHANGES.ext rows**, codes unchanged; en: 0 Dutch words (except the host-requested nl test notification); live switch by rename, in-place rewrite and back, and a folder created after `enable()` each give exactly a fresh start's 55 rows; `disable()` leaves 0 monitors and 0 timers; a mutant without the `installed-changed` handler fails the switch test.
* **Private GNOME Shell 50.1** (`tools/heavy.sh --min-mb 2560 --tag ge1c-scenario -- python3 -B tests/gnome/harness.py launch run --lang nl --scenario tests/gnome/scenarios/ge1c-extension.py`, worktree source, baseline binaries): install ok (87,844 paths, 0 outside), extension ACTIVE, **25 checks, 23 pass, 2 known** (`C6-runtime`: the pinned File Explorer button keeps "Win11 Verkenner" after a live switch because the 3b898e6 panel has no `Name=` rewriter yet). Accessible names, Ctrl+Alt+Tab group, clock format and a bridge failure text switch by rename and by rewrite, and on the fallback bar after `kill_panel`. 101 s, peak PSS 829 MiB.
* **Widths (measured only)**: fallback search button 178 px in both; label 58 px nl / 56 px en; never compact at 1440 px.

## 3. Deviations and notes

1. I ran `gnome-shell --version` once (prints the version, no session) while looking for GNOME's JS; afterwards I only read `libshell-18.so` resources with `gresource extract` (ctrlAltTab.js, appDisplay.js).
2. `uninstall.py` ran only inside private.sh + bwrap with `--files-only` (keyfile GSettings, stubbed gnome-extensions).
3. The Node extension test lives in `tests/installer/ext/` (the only test directory GE1c owns).
4. `tests/gnome/scenarios/example.py` (GE1h) marks "the extension's labels are Dutch only" as known GE1c; those checks should now pass.

## 4. Decisions for the coordinator

1. **H fallback for other keys.** C1 is applied to the language only; `readSettings()` still falls back to `~/.config/win11shell` for theme/taskbar/pins/alignment when the G file is missing (F20's inconsistency). The installer now always creates the G file on a first install, so it is nearly dead; dropping it is one line.
2. **"Win11 voor GNOME" in English** notifications and the uninstall message is "Win11 for GNOME" ("voor" is Dutch). D-G16 keeps the `metadata.json` name; confirm the reading.
3. **`--lang` on a stored choice** is ignored with a note (C6 as written). H let an explicit `--lang` change it. If wanted, the banner could then say `./install.sh --lang en`.
4. A first install stores nl too (H stored only en); an H payload without H settings counts as nl.
5. libarchive.so.13, python3-gi and glib-compile-schemas are now hard requirements of the installer (exit 1 with an apt line); update-desktop-database is a warning.
6. Commit `tests/gnome/scenarios/ge1c-extension.out/` (3 PNGs, 120 KB) or not.

## 5. Left

* GE1b's runtime C6 rewrite (win11-panel) for the two known scenario checks; rerun the scenario with `--build-dir target/debug` after GE1b.
* The extension changes reach Robin's session only after the round-7 re-login.
* NucBox: `tools/h-remnants.sh` (Robin), then his clean-up decision.
* README/docs for `--lang`, the preflight and h-remnants (not GE1c's files).

## 6. Fix round (2026-11-05, after `ge1c-review.md`)

Only GE1c's allowlist was edited. sha256 of every owned file was taken at the start
(`tests/installer/run/ge1c-fix-start.sha256`, ignored dir); at the end exactly the 14 files below differ, all mine.
Dutch is unchanged: the nl extension dump still differs from 3b898e6's in exactly the 38 `key:ext.*` rows, and
Robin's update case is still byte-identical (stdout, stderr, tree) to the 3b898e6 installer.

### Findings and what I did

| Finding | Action |
|---|---|
| **M1** English failure texts | Fixed. Only `en` values changed (nl, codes, BASELINE rows unchanged): the review's table for `startMissing`, `sessionApps`, `appMissing`, `panelMissing`, `badHost`, `hostUnmapped`, `hostUnpainted`, `badGeometry`, `waitFrame`, `windowGone`, `explorerGone`, `cantMinimize`, `cantMaximize`, `cantClose`, `unknownMethod`, `badSnap`, `offline`, `badPid`, `badIdentity`, `identityMismatch`. Three more for the same style: `cantResize` "This window can’t be resized.", `badRects` "Icon rectangles must be a list." (nl says "lijst"), notifications `unknownMethod` "Unknown notification function: {method}" (nl "meldingsfunctie", like the panel's "GNOME function"). Notifications `offline` is "The notification link is turned off." instead of the review's "bridge" (jargon, the very thing M1 is about; nl "koppeling"). The table comment in `panelBridge.js` no longer says "en is the 3b898e6 text". Nothing matches on these texts (grep of `vendor/ui/js`, `host/src`: no consumer; C2). I read all 33 en failure rows of the dump. Scenario `FAILURE['en']` updated. |
| **m1** interrupted first install loses the language | Fixed. `install.py:main` now calls `store_language()` (and prints the language line) after every check (preflight, conflicts, config folder, icon and portal preflights) and before the first change (the stage). A failure before that point still stores nothing. New test `test_interrupted_first_install_keeps_the_language`: en_GB locale, a `glib-compile-schemas` stub that exits 1 → run 1 exits 1 with the payload in place, first line "Language: English. …", a complete en `settings.json`; run 2 exits 0, `settings.json` byte-identical, no second banner, 0 Dutch words, 10 English `Name=`; plus a conflict refusal that leaves the tree unchanged and stdout empty. Mutant check: with the old order the test fails (no banner, no file). |
| **m2** no glossary rule in `check_pairs.py` | Fixed. `check_pair()` applies `check.Glossary` over `tools/i18n/glossary.tsv` (the same class `check_ui.py` uses) and an en-GB spelling rule (`minimized`, `color`, `center`, `canceled`, …); both are warnings until the round-3 lock-in, like the UI's. On purpose in `GLOSSARY_OK`: the Dutch UI path in `T.langZone`/`T.langFormats` (printed only in nl) and "achtergrond" = backdrop in `TEXT.identityMismatch`. Without that list the checker gives exactly those 7 warnings; with it 0. On the pre-fix texts the glossary flags `panelMissing` (Taskbar) and `explorerGone` (File Explorer) and the spelling rule flags `minimized`/`maximized` (the glossary alone cannot: nl "geminimaliseerd" is not the glossary's "Minimaliseren"). `test_pairs_checker_is_clean_and_catches_errors` now asserts both warnings on a bad table and none on the tree. |
| **m3** silent deletion of unknown files in the extension folder | Fixed (named, still removed). `install_extension()` prints `staleRemoved` for every top-level file it removes ("Verwijderd uit de extensiemap (geen bestand van deze versie): {path}" / "Removed from the extension folder (not a file of this version): {path}"); `uninstall.py` prints the same for every removed file that is neither in 3b898e6's 9 names nor in this version's `extension/`. Printed only when such a file exists, so Robin's update and uninstall outputs stay identical (tests). New row `key:install.staleRemoved`; the three behaviour rows are updated. Tests: reinstall with a removed module and a foreign `ge1c notes.txt` → both named, in nl, before the closing lines; in en after a switch; uninstall with a foreign `ge1c-unknown.js` → named, then the usual line. The round-7 backup step is not GE1c's file: see decision 8. |
| m4 throttle, not debounce | Fixed. `LanguageWatch._queue()` removes the pending timer and starts a new one on every event. The store's rename is one event, so the switch still takes 100 ms (scenario: 100-111 ms). New Node test `test_slow_writer_is_read_once_after_its_last_event` (`dump.mjs --burst`: half-written JSON, then more, then the whole file): one pending timer at a time, a new id per event, 0 reads during the burst, the result equals a fresh en start. Mutant check: the old throttle keeps one id for all three events and fails. |
| m5 one `destroy` handler per switch | Comment in `_relabel()` (GNOME 50 `ctrlAltTab.js` behaviour, harmless; reaching into `_items` would be worse). No code change. |
| m6 unreachable en of `langZone`/`langFormats`; quote nit | Kept for the checker's parity, with a comment saying why. `badLang` en now quotes ‘…’ like nl (en-GB); `test_arguments` updated. |
| m7 `build.sh` not offline, 4 jobs | Fixed: `cargo build --release --offline --locked -j "${WIN11_BUILD_JOBS:-2}"` (C8, F34); messages unchanged (the nl/en build.sh test still passes); behaviour row added; a unit test pins the line. |
| m8 "System" header of Main.notify | No change: changing the source would change nl output (D-G11); for Robin (decision 10). |
| m9 GNOME's ~5 s lag on a `Name=` rewrite | Scenario: the C6-runtime check now polls 12 s (`c6_runtime()`), and it is a hard check with any `--build-dir` but `3b898e6-debug`, so the rerun after GE1b only needs the build dir. |
| m10 scenario gaps | Done by the review; kept. |
| m11 `gnome-shell --version` | No action (recorded in §3.1; nothing was run outside the sandboxes in this round). |
| Cross-layer `theme.js` C1 | Not my file. Still present: the scenario records `lang="EN"` and `lang="[object Object]"` on the panel page for the two edge values (`c1-*.panelLang`). For GE1a (decision 7). |

### Files changed in the fix round

`extension/panelBridge.js`, `desktopBridge.js`, `frostedBridge.js`, `notificationsBridge.js` (en values, comment),
`extension/extension.js` (debounce, comment), `scripts/install.py` (store early, `staleRemoved`, `badLang` en,
comment), `scripts/uninstall.py` (names unknown files), `scripts/build.sh` (offline, `-j2`),
`tools/i18n/check_pairs.py` (glossary + en-GB rules), `tests/installer/test_installer.py` (m1 test; m3, m6, m7, m2
assertions), `tests/installer/test_extension_text.py` (m4 test), `tests/installer/ext/dump.mjs` (`--burst`),
`tests/i18n/BASELINE-CHANGES.ext` (`key:install.staleRemoved`, 3 behaviour rows updated, 1 added),
`tests/gnome/scenarios/ge1c-extension.py` (en failure text, `c6_runtime()`); outputs
`tests/gnome/scenarios/ge1c-extension.out/` (replaced by the fix run) and `ge1c-extension-r1.out/` (new).

### Verification (from G)

* `python3 -B tools/i18n/check_pairs.py --warnings` → **0 errors, 0 warnings**, 144 pairs (one new: `staleRemoved`),
  144 glossary-checked. `python3 -B tools/i18n/check.py` → 0 errors, 52 warnings (all UI glossary, GE1a).
* `node --experimental-default-type=module --check extension/*.js` → 6/6 ok.
* `W11_INSTALLER_REAL=0 tests/installer/run.sh test_extension_text` → **8 tests OK in 1.4 s** (also after the last
  comment edit).
* `tests/installer/run.sh` (heavy.sh `--min-mb 1024` + private.sh + bwrap) → **37 tests OK in 236.0 s** (3 min 57 s
  wall): the 35 before plus the m1 and m4 tests. Real nl update: 140,216 mutating syscall paths, 0 outside the case
  dir, byte-identical to 3b898e6; real en first install: 140,223 paths, 0 outside.
* Scenario, baseline binaries: `tools/heavy.sh --min-mb 2560 --tag ge1c-fix -- python3 -B tests/gnome/harness.py
  launch run --lang nl --build-dir tests/build/3b898e6-debug --scenario tests/gnome/scenarios/ge1c-extension.py` →
  **rc 0, 64 checks, 63 pass, 1 failure marked known** (`C6-runtime`: the 3b898e6 panel has no rewriter; polled
  12 s, GNOME kept "Win11 Verkenner"); 143.5 s; peak PSS 920 MiB; min MemAvailable 4,212 MiB; install 79.7 s,
  87,844 mutating paths, 0 outside `run/`, 12/12 binaries and 9/9 extension files identical; no leftover processes.
  Re-label after the file's mtime: dropdown 111 ms, rename 100 ms, rewrite 100 ms, "EN" → nl 100 ms, object form →
  nl 102 ms. End-to-end failure toast: nl "Het gevraagde app-venster bestaat niet meer.", en "The requested app
  window no longer exists." (`en-dropdown-failure-toast.png`). Simulated C6 rewrite: pinned button followed in
  5.49 s (en) and 4.89 s (nl). Fallback bar shown 0.14 s after the panel stopped, English, then nl and en again.
  The comment-only edit of `extension.js` (debounce wording) came after this run's install; the r1 run below
  installed the final file.
* Scenario, GE1b's round-1 binaries (extra, to turn C6-runtime into a hard check): `tools/heavy.sh --min-mb 2560
  --tag ge1c-fix-r1 -- python3 -B tests/gnome/harness.py launch run --lang nl --build-dir tests/build/r1-debug
  --out tests/gnome/scenarios/ge1c-extension-r1.out --scenario tests/gnome/scenarios/ge1c-extension.py` (the lock
  waited for another step's harness run, 16:21-16:24) → **rc 0, 64 checks, 64 pass, 0 failures**; 125.7 s; peak PSS
  932 MiB; install 74.6 s, 87,844 paths, 0 outside, 12/12 binaries (r1-debug `SHA256SUMS` verified first) and 9/9
  extension files identical. **C6 end to end:** after the dropdown switch, win11-panel's rewrite reached the pinned
  button as "Win11 File Explorer" within 4.95 s of the poll start (hard check). Re-label 110/100/102 ms, "EN" →
  nl 102 ms, object form → nl 100 ms; en failure toast as above; fallback bar 0.15 s. In this run the simulated
  rewrite to the English name found it already there (0.0 s), and the simulated rewrite back to "Win11 Verkenner"
  showed after 5.89 s.

### Contract deviations

None. C1 (extension reads only Win11's file, creates nothing, follows all write paths), C3 (inline tables, codes
unchanged), C5, C6 (install time in the stored language; at run time end to end with GE1b's panel), C7, C8 (every
heavy run through heavy.sh; cargo is not run by GE1c; `build.sh` now follows C8 too). The UI's C1 gap
(`theme.js`) is GE1a's and still open.

### Decisions for the coordinator (the review's six plus four)

1. Keep the `readSettings()` fallback to H's settings in round 1; decide after `tools/h-remnants.sh` has run here
   and on the NucBox (review §6.1).
2. "Win11 for GNOME" in English: ask Robin together with the `metadata.json` name (F31).
3. Keep ignoring `--lang` when a language is stored (C6 as written; the note says where to change it).
4. Accept that a first install stores nl too, and that the complete defaults document fixes every default at
   install time (new keys still merge).
5. Accept the hard installer requirements (libarchive.so.13, python3-gi, glib-compile-schemas); they pass here.
6. Commit `tests/gnome/scenarios/ge1c-extension.out/` (836 KB, fix run on 3b898e6 binaries) and
   `ge1c-extension-r1.out/` (728 KB, run on `tests/build/r1-debug`).
7. Route the `theme.js` C1 fix (`"EN"` → panel stays en with `lang="EN"`; object → `lang="[object Object]"`) to GE1a.
8. **New (m3):** add `~/.local/share/gnome-shell/extensions/win11-gnome@win11shell.local/` to the round-7 backup
   step (F22) next to `~/.local/bin/win11-*` and `~/.local/share/win11-gnome`; the installer now names what it
   removes there, but the content of a foreign file is still gone.
9. **New (m7):** `scripts/build.sh` is now offline with 2 jobs by default; a release build on this PC is slower and
   needs a filled cargo cache. `WIN11_BUILD_JOBS` overrides the jobs; the first release build should record its peak
   RSS (F34).
10. **New (m8, Robin):** the "System" source header of the extension's own GNOME notifications comes from GNOME's
    locale in both Win11 languages; an own `MessageTray.Source` would change nl output.

### Left

* The `theme.js` C1 fix (GE1a), then a rerun of this scenario.
* After GE1b's fix round: rerun with `--build-dir target/debug` (or a new snapshot); C6-runtime is already a hard
  check for any build dir other than `3b898e6-debug`.
* README/docs for `--lang`, the preflight, `staleRemoved` and h-remnants (not GE1c's files).
* The extension changes reach Robin's session only after the round-7 re-login.
