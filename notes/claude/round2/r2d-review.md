# R2-D review: apps and native glue (round 2, 2026-11-05)

Adversarial review of the uncommitted R2-D work in W = `.wt/r2-d` (branch `r2/d`, HEAD d666df54). I edited no
product file. Scratch scripts and evidence are in `W/.rt/review-r2d/`. No win11-* binary, gnome-shell, installer or
gsettings/dconf ran outside the sandboxes. Every cargo run went through `tools/heavy.sh` (tags `r2d-rv-*`). I did not
rerun the GNOME harness: it cannot start from a worktree (R2-D's §8.1, verified), and a copy would have to sit two
levels below W, outside the paths I may write.

**Verdict: pass.** I found no blocker and no major. Dutch is byte-identical at all 81 converted sites. Every done-when
check is green. The six extras are done and correct. What remains is six minors and some notes. The most important
one is a one-off deadlock of the sandboxed lib tests (1 run in 18). It is probably not caused by R2-D's code, but
run-tests.sh has no timeout, so the hang kept the machine-wide lock for 7.5 minutes until I killed it.

## 1. Checks I ran myself (final tree)

| Check | Result |
|---|---|
| `drift_rust.py --paths <the 16 owned files> --warnings --summary` | 1,041 present, 205 mapped or listed, 96 rows, 100 new literals (95 technical, 4 bilingual-pair key names, 1 log); **0 errors, 0 warnings** |
| `drift_rust.py --warnings --summary` (whole tree) | 13,450 frozen; 13,245 present, 205 mapped or listed; ratchet 274 new (none user-visible in R2-D's files: the 3 user-visible-dutch and 1 user-visible-english are i18n.rs's own log lines); **0 errors, 0 warnings** |
| `check_rust.py --warnings` (node on PATH, labels checked) | 24 stems, 96 rows; **0 errors, 0 warnings** |
| `check.py` | **0 errors**, 52 glossary warnings, none in R2-D's stems |
| `heavy.sh --tag r2d-rv-check -- cargo check --offline --locked -j2 --all-targets` | rc 0. Warnings: lib 28, lib test 26, win11-launch 2, win11-launch test 1. That is the baseline. Those in owned files are all pre-existing (pertype.rs:10, picker.rs:35 `show_portal`, launch_context.rs:95/:114) |
| `heavy.sh --min-mb 2048 --tag r2d-rv-test -- run-tests.sh --no-fail-fast` | **Run 1: the lib test binary deadlocked** (finding m1). I killed it after 7.5 min, rc 101. **Run 2** (`r2d-rv-test2`): rc 0, lib **281 passed, 8 ignored**. win11-launch 16 passed, 1 ignored. Oracles **16/16** (shell `oracle_tests` 8, the 7 disks python cases, `reference_contracts` 1). `every_converted_site_renders_its_3b898e6_dutch` ok in lib and bin. `guard-calls.log` 0 bytes. Then 16 more runs in one heavy command (`r2d-rv-repeat`): 8 full (281 passed each) and 8 skipping R2-D's 3 new catalog tests (278 each). **No hang in any of them.** |
| Build snapshot | `tests/build/r2d-debug`: SHA256SUMS 12/12 OK. Every binary `cmp`-equals `target/debug`. Built at 18:34, after the last source or TSV edit (18:31). |
| Harness evidence (read, not rerun) | The results.json files agree with the note. `ge1a-switch.r2d`: 61/61, fastStart nl lastDirty 321 / en 322 ms. `-2`: 61/61, 335 / 321 ms. `ge1a-start-switch.r2d`: 16/16. `r2d-picker.out/r2d`: 10/10, both pickers `io.github.win11shell.FilePicker`. `r1`: both `io.github.win11shell.Settings` (marked known). `inner.facts.systemBus` names `W/t/g/run/rt/system-bus`, which confirms the copy workaround. |
| Ownership | `git status --short` lists only owned paths: the 5 stems, 12 host/src files, the allow and baseline files, `r2d-picker.py`, the scenario outputs and `notes/claude/round2/`. `git diff --quiet d666df54` holds for i18n.rs, lib.rs, window.rs, runtime.rs, services/mod.rs, files/, api-methods.json, events.rs, Cargo.*, vendor, extension, scripts, tools, the runners, core.tsv and window.tsv. `wallpaper.tsv` is untouched. G's own tree is clean. `W/t`, `tests/gnome/run` and `tests/build/ref-r1-debug` are gone. |

## 2. Dutch byte-identity (check 1)

* **Every site, not a sample.** `.rt/review-r2d/pair_sites.py` walks `git diff -U0 3b898e6` of the 11 changed .rs
  files. Each removed literal is paired with the catalogue key that replaced it in the same hunk. **81/81 removed
  literals** match their key's nl, or en for the 2 English halves (`File Explorer`, `{s} links`), byte for byte. The
  full list is in `.rt/review-r2d/pairs.tsv`.
  * The only hunk with renamed placeholders is pertype.rs:96, `{s}` → `{scheme}` with `"scheme": s.to_string()`. I
    checked it by hand.
  * The other mismatches the script reports are test-module hunks.
  * Because the pairing is per hunk, a swap of two keys between call sites would show up. The drift test alone would
    not catch that.
* **Format rows rendered with real arguments.** `.rt/review-r2d/render_fmt.py` emulates `i18n::fill` and Rust's
  `format!` for the 6 format sites: `gnome.sessionUnreachable`, `invalidJson`, `rectNoValue`, `regionFailed`,
  `launch.hostNotFound` and `apps.schemeLinks`. It uses 5 values each, including values that contain `{error}` or
  `{key}` themselves (fill makes a single pass). **30 renders, 0 differences.** Each call site passes
  `expr.to_string()`, the same Display text `format!` printed.
* **The nl render test through call sites** covers about 30 of the sites, with 3b898e6 bytes typed in the tests. All
  pass.
* **`BASELINE-CHANGES.rust-r2d`** (62 lines): none of them is a Dutch change. I checked each class against
  3b898e6:
  * **57 `nlNames` values and 1 `twins` nl**: 3b898e6's `nlNames` equals `files/launcher-names.json` `NL_NAMES`, same
    keys, values and order. `twins` equals `SAME_NAME_AS_OURS`. launcher-names.json is unchanged since 3b898e6, and
    the rest of catalog-data.json equals 3b898e6 plus round 1's "Alles" fix.
  * **2 JSON keys**: `nlNames` and `twins`.
  * **2 technical `"en"` literals**: the `lang == "en"` tests of the two rewritten pairs.
  * These are the gate's §5 "removed technical/moved literal" class, and every reason is true. There is no D-G1a or
    D-G2 line, which is correct: no R2-D text has a count, and none is GNOME-wrong.
* **Error codes** (`ApiError::new|tr("<code>"`, outside tests) per file are identical to 3b898e6 in apps/mod.rs,
  pertype.rs, defaults.rs, recent.rs and gnome.rs.

## 3. Findings

| # | Severity | Finding | Evidence | Fix |
|---|---|---|---|---|
| m1 | minor (flaky, probably not R2-D's) | The sandboxed lib tests deadlocked once in 18 runs. run-tests.sh has no timeout, so the hang kept the machine-wide heavy lock until I killed my own test binary after 7.5 min. | Run 1 (`r2d-rv-test`): 77 of 289 tests ok, then 16 tests "running for over 60 seconds". They include 7 `apps::catalog` tests, 3 of them R2-D's new ones, plus `apps::recent` ×2, `files::places`, `files::properties` ×4, `files::actions` and `app_params`. In `/proc`, every stuck thread is S (futex), except 7 threads of the concurrent-build test waiting on that test's own `.apps-seen.lock` flock, held by a thread that is itself futex-blocked. Each other catalog test holds its own fixture's flock while futex-blocked inside `build()`. The common path is `own_value` → `own_icon` → `files::icon_exists` → `read::icon_catalog` → `current_theme()`, which creates a `gio::Settings` on every call. The process has an inotify fd and a `gmain` worker, so GSettings did start. Snapshot in `.rt/review-r2d/hang-1.txt`. The 17 runs after it all passed in about 1.5 s, 8 of them without R2-D's 3 new tests. | No R2-D change is needed. Coordinator: give run-tests.sh a hang guard as a tool fix, for example `timeout -k 10 600` around `cargo test`, so that a hang can never hold the shared lock indefinitely. Rerun on the merge. Owner of `files/read.rs`: `icon_catalog()` calls `current_theme()`, which builds a GSettings, before it checks its 30 s cache, in every thread. Checking the cache first, or caching the theme, removes that from the concurrent cold path. |
| m2 | minor | The allow-list rules for whole files are wider than their reason (C9: narrow). | `allow-host-r2d.txt:15` `wallpaper.rs class:user-visible-dutch`, `:25` `portal.rs class:user-visible-english` and `:26` `portal/external.rs class:user-visible-english`. A user text added to these files later would pass the ratchet unseen. The portal reason ("stderr or a D-Bus error") is also not true for `portal.rs:538` "Nested file chooser unavailable in portal": that is an `Err` on a UiRequest ack to the picker page. No page sends a FilePicker or Regions request inside the portal, so it is unreachable, but the reason should say so. | Use `fn:`/`text:` selectors: wallpaper `fn:image`, `fn:target`, `fn:sync` (the 7 texts); portal.rs the 3 `text:` rules plus `text:Win11 FileChooser parent`; external.rs `fn:new`, `fn:apply`. Correct the reason for the nested-picker ack. |
| m3 | minor | The same Dutch has two different English forms in the UI and the host. | `apps.tsv:45` `apps.defaultNotChanged` en is "Couldn’t change the default app." The UI's `saveFailed` for the identical nl "De standaard-app kon niet worden gewijzigd." is "The default app couldn’t be changed." (`vendor/ui/js/apps/defaultapps/main.js:35`). In English, Default Apps can show both for the same failure: its own toast, and the host's error message. | `extract_rs.py en apps.defaultNotChanged "The default app couldn’t be changed." --apply`. `apps.somethingWentWrong` follows the glossary ("Try again"), while the UI says "Please try again". Leave it, or align it in the round-3 glossary lock-in. |
| m4 | minor | Two English wordings are weak. | `gnome.tsv:22` "Invalid reply from the GNOME link": "link" for GNOME-koppeling (the extension bridge) is opaque. `apps.tsv:69` "… Run install.sh again, then try again." says "again" twice (it is H's text with ’). | `gnome.invalidReply` → "Invalid reply from the GNOME connection". `apps.explorerNotInstalled` → "File Explorer isn’t in the app menu yet. Run install.sh again and then retry." |
| m5 | minor | F38 has residual gaps, and its tests miss them. | (a) `english()` returns None when an entry has only localised keys, so the system-language text stays in English. A GLib probe under `LANG=nl_NL.UTF-8` (`.rt/review-r2d/keyfile_probe.py`, KEEP_TRANSLATIONS like `load_keyfile`): `GenericName[nl]` without `GenericName=` gives an error for en_GB, so `catalog.rs:358` keeps GIO's Dutch "Alleen NL". (b) An empty `Name[en_GB]=` is filtered to None and falls back to GIO's Dutch name, not to `Name[en]`/`Name=`. (c) The note says the order is "Name[en_GB], then Name[en], then Name=". The code first takes `translator.get(…, "en")`, so `Name[en]` (when it differs from `Name=`) beats `Name[en_GB]`. | (a) In English with a translator, fall back to `""` for generic and comment instead of GIO's text when the entry has no untranslated key. Keep GIO's name for the name, since a name must not be empty. (b) Try `Name[en]`/`Name=` explicitly when the en_GB value is empty. Add a fixture row for each. Correct the order in the note. |
| m6 | minor | The picker o2 fix changes how a non-modal in-app picker looks to the taskbar, and the scenario does not check that. | `r2d-picker.out/r2d`: the NORMAL picker has `skipTaskbar: false`, so panelBridge's `_userWindows()` lists it. Its `_windowJSON` now gets `appId` `io.github.win11shell.FilePicker` and `desktopId` from WindowTracker (`io.github.win11shell.Settings.desktop`) instead of the map's `win11shell-settings.desktop` (`panelBridge.js:794-806`). `shell.js` `resolve()` may then miss Win11 Settings and draw a separate button. No Win11 caller opens a non-modal picker; a portal caller with `modal:false` can (GtkFileChooserNative's default). The modal picker is unchanged. | Before accepting decision 3, add a check to `r2d-picker.py` that reads panelBridge's window list (`windows.list`) for the non-modal picker. Decide it together with the shadow question in round 3 (extension side). |
| n1 | note | The language source for service callers is `current_language()`. | `launch::command` (services/shell.rs), `gnome::call_api` (runtime.rs:171, notifications.rs:458) and `process_scope::spawn_pid*` (shell.rs:205, sysmon/actions.rs:883) read the process file, not `ctx.lang()`. In production that is the same file (`i18n::config_dir()` = the ctx's config dir). It differs only in a service test with its own config dir. The callers belong to other steps. | Later: `lang`-taking variants (`command_in` already exists), passed `ctx.lang()` by the services' owners. |
| n2 | note | The Monitor can rebuild twice on a language change. | `apps/mod.rs:232-243`: on `changed`, `invalidate()` and then `list(next)`. If a page's `apps.list` already rebuilt the catalogue for the new language (per-call `language_in`), the invalidate forces a second full rebuild and a second `apps.changed`. `applied` tracks the monitor's last reading, not the catalogue, so an en→nl→en flip between two readings sends no event. That is harmless, because every `apps.list` reads the language itself. | `if due { invalidate }` and `if changed { list(next) }`: `ensure()` already rebuilds on a language mismatch. |
| n3 | note | The fallback text is rendered eagerly. | `gnome.rs:27` and `:205` `.unwrap_or(&tr(…))` render the fallback on every error, even when the extension sent a message. | `unwrap_or_else`, or a `match`. |
| n4 | note | Two statements in the report are inaccurate. | "None was copied from H": 12 `apps` rows are H's en with ’ (for example `apps.explorerNotInstalled` = H `apps.setDefault.notInstalled`, `apps.fileGone` = H `apps.fileNotFound`). That is allowed (exact nl match), but the sentence overstates it. The note's §5.1 order is covered under m5(c). | Reword the note. |
| n5 | note | The harness ran from a copy. | It was disclosed and is consistent with results.json (`W/t/g/run/…` socket path). The byte-identity of the deleted copy cannot be re-verified now. The ge1a outputs sit outside the `r2d-*` pattern, as the task named them. `r2d-picker.py` decides "known" from the build path (`'r2d' in build`). | Coordinator: apply the `'system-bus'` → `'sbus'` patch (harness.py :125, :905, :917, :1387; I verified 102 → 96 bytes), then rerun the three scenarios from W with the real harness. |

## 4. What I verified and found correct

* **Language (check 2).**
  * apps uses `i18n::language_in(&self.config)` once per call. That is the same directory `files.context()` read,
    and Apps never runs with `WIN11_REFERENCE_INSTANCE`. recent uses `Lang::of_settings` of the document it already
    reads. picker, transfers, gnome, launch, process_scope and the win11-launch bin use `current_language()`.
  * win11-launch keeps `WIN11_GNOME_CONFIG_DIR`/`XDG_CONFIG_HOME`: `launch_context_for` drops only
    `WIN11SHELL_`/`WIN11_DEV_`/`WIN11_TEST_`.
  * No text is frozen.
    * pertype's `kinds()` cache is keyed by `language`, and so is the catalogue state.
    * `data()`/`launcher_names()` hold data, not rendered text.
    * `own_icon`, recent's cache, `PENDING` and the counters hold no text.
    * No text is stored for later.
* **Missed sites (check 3).** `extract_rs.py list` shows 0 remaining in every owned file except `backend.rs`, whose 18
  texts the gate leaves out ("internal reference backend"). `--all` shows only allow-listed, technical and log
  literals.
  * The allow-list claims are true. `Appbewaking`/`Tab-overdrachtsmap` go to `Runtime::new` and then `eprintln`.
    `ensure_slice` runs only for `Role::Shell`, from `ensure_native`. The two defaults.rs io::Errors are always
    replaced (`map_err(|_|…)`, `.unwrap_or(false)`). wallpaper reaches only `settings_runtime.rs:139` `eprintln`.
    The two CLI usage errors come before the ack socket exists.
  * A grep for English sentences in the owned non-test code finds only allow-listed Dutch.
* **English (check 4).**
  * I read all 69 rows. Every apostrophe is ’, and there is no ASCII `'`. The final full stop matches nl in every
    row. Placeholders match. There are no plurals and no quoted UI labels. There is no Dutch (`[en-dutch]` is
    clean), and Win11Shell is kept (D-G16).
  * `File Explorer`, `Choose an existing folder.` and `mailto links` equal the UI's English.
  * Except for m3 and m4, the wording is good en-GB ("realised", "any more").
* **Behaviour (check 5).**
  * The UI never matches on these messages or keys. grep finds no `e.key`/`message` match for apps, picker,
    transfers, launch or gnome, and the UI's code matches (`unknown_method`, `cancelled`, …) are not R2-D's.
  * No Rust code outside the owned files matches these Dutch texts. The hits are the UI's own dictionaries or mocks.
  * C2 envelopes keep their code and message and gain key and params.
  * The oracles are untouched and green.
* **Extras (check 6).**
  * **1 (F38).** Implemented as described. Dutch on en_US, Dutch on nl, and English on en keep the old path,
    including the hidden-entry branch. The fixture tests cover both system locales. Residual gaps under m5.
  * **2 (Start tiles).**
    * One stat per poll instead of a full read.
    * A GFileMonitor on the config directory, created inside `with_thread_default`, so its signals run on the
      catalogue thread, not the GTK thread. WATCH_MOVES with both `file` and `other` checked catches the store's
      `.settings-*.tmp` → `settings.json` rename (settings.rs:361-375).
    * `apps-seen.json` writes are filtered out, so there is no feedback loop. The monitor is cancelled at shutdown.
    * The measured times are as reported. The derivation of the 300 ms floor (`shell.js:159`
      `debounce(load, 400)`) is sound.
  * **3 (dedupe).** Exactly equivalent (§2). R2-C has not touched `launcher-names.json` and expects the dedupe
    (r2c.md:180). `gnome_names_come_from_verkenners_launcher_table` guards the keys and the 57-entry count against a
    later restructure.
  * **4 (o2).** The map-signal analysis matches GTK3: `gdk_wayland_window_set_application_id` is a no-op before the
    xdg_toplevel exists, and the handler runs on every map. The harness proves the class. The extension's own
    comment (frostedBridge.js:10-17) intends exactly this identity. Side effects under m6.
  * **5.** `mod i18n` is in bin/launch.rs. The bin tests now run 16 (10 i18n plus 1 new plus 5 old).
  * **6.** "Alles" is unchanged in both languages.
