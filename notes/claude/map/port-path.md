# Port path: H's English work into G (read-only map, 2026-11-05)

G = `/home/robin/Documents/Codex/2026-11-01/chat/outputs/win11-gnome` (branch `claude/gnome-english-update`, clean, = `3b898e6`).
H = `/srv/codex-claude/claude-scratch/c2c10c4c-ae6d-4f6a-a907-342de497d54f/931db6b9-2976-4e14-8e5a-041952887025/scratch-2026-10-25-a089b6`.
B = the 161-file base that G vendored (identified in F1). Line numbers below are as of 12:38 on 2026-11-05.

Nothing in G or H was changed. The only writes are this file and `notes/claude/map/merge-trial/` (see F4). Scratch
copies (merged tree, scans) are in the session scratchpad and are not needed again.

## Summary

* **G vendored the 2026-10-27 interim UI.** All 161 baseline hashes in `G/vendor/ui-original-sha256.json` match
  Robin's installed `~/.local/share/win11shell/ui` (161/161) and both `H/dist/win11shell-2026-10-27-interim*.tar.gz`
  (161/161). H's UI today matches on 117 of 161 files. The base is older than the services split and older than every
  E-step.
* **H changed 44 UI files on top of that base, all of them JavaScript.** No CSS, no assets and no HTML changed, so the
  English work does not touch the Windows 11 look. 40 of the files are English-option work only (3 E0a, 33 E1,
  4 E1 plus the E-review fixes). One file holds services-split work only (`taskmgr/hostbanner.js`). Three files mix
  split and English work (`taskmgr/hostcall.js`, `taskmgr/pages/details.js`, `taskmgr/store.js`).
* **Codex changed 24 base files and added 15.** 11 of the 24 are also changed by H. The 15 added files have no H
  counterpart.
* **The 3-way merge (B to H, B to G) is almost clean.** 175 of 176 files merge without conflict: 104 unchanged,
  33 H-only, 13 G-only, 15 G-added, and 10 of the 11 overlaps. One file conflicts: `js/desktop/desktop.js`, one
  hunk. Both sides rewrote the same `onLangChange` line, and the fix is to keep both
  (`refreshView(); reloadSoon();`).
* **The split code can be kept out cleanly.** Merging the three mixed Taskmgr files with the 2026-11-04 tarball as the
  base drops the direct-worker routes and keeps only the E1 hunks. All three merge clean, and `hostbanner.js` stays as
  G has it. A candidate set of 43 files is in `merge-trial/proposed/`. In the full merged tree, all 133 JS modules
  pass `node --check`.
* **H's checker and UI scanner already run on G's UI.**
  * `check.py`: 0 UI errors. G has 72 `makeT` dictionaries with 2,638 keys at nl/en parity, so Codex's new UI is
    already bilingual.
  * The merged tree adds no warning beyond H's 52 glossary warnings.
  * The ratchet scan finds 7 Dutch-only literals that Codex introduced (6 real, 1 false positive). Every H-allowlisted
    hit survives in the merged tree.
* **The UI is the small part.** G's Rust host (55.7k lines in `host/src`) has:
  * 251 `ApiError::new` sites in non-test code, 203 of them with a literal Dutch message;
  * about 630 literals with a Dutch word, of which about 274 match no H catalog row (G-only texts, plus logs).
  H's host catalogs (`i18n/host/*.tsv`, 588 rows) are a strong starting point: for 507 of 588 rows, the longest nl
  fragment occurs verbatim in G's Rust sources or JSON tables.
* **What carries over, and what does not:**
  * Reusable as-is or nearly so: the UI tooling (`jsdicts.mjs`, `tok.mjs`, `scan_ui.mjs`, the UI half of
    `check.py`/`ratchet.py`, the glossary, `dutch-words.txt`, the dump harness).
  * Reusable after a path change: the dump harness (`vendor/ui` instead of `ui`), which also needs new surfaces and
    mocks for 18 G-only API methods.
  * Not reusable: the Python-host parts (`host/i18n.py` runtime, `scan_py.py`, `extract_py.py`, the Python AST
    rules) and the bash parts (`lib.sh`, `scan_sh.py`, `extract_sh.py`, `install`/`uninstall`/`session` catalogs,
    transcripts). G needs a Rust runtime and a Rust scanner instead.
* **Estimate: about 52–58 agent-hours to make the whole GNOME version bilingual.** UI port about 5.5, tooling about
  15, Rust host plus installer and docs about 31–36.

## Findings

### F1. Which UI G vendored (step 1)

Method: sha256 of each of the 161 paths in `G/vendor/ui-original-sha256.json`, compared with the same path in each
candidate (`cmp.py`/`chain.py` in the scratchpad).

| Candidate | match | differ | absent | Note |
|---|---|---|---|---|
| `~/.local/share/win11shell/ui` (installed interim, read only) | **161** | 0 | 0 | also exactly 161 files, no extras |
| `H/dist/win11shell-2026-10-27-interim.tar.gz` (`win11shell/ui`) | **161** | 0 | 0 | the same for `…-interim-plugin1.6.tar.gz` |
| `H/dist/i18n-snapshots/E0a/ui` (pre-E0a copies) | 3 | 0 | 158 | only `core/i18n.js`, `core/theme.js`, `mock/settings.js`; all equal B |
| `H/dist/i18n-snapshots/E1/ui` (pre-E1 copies, 05:17) | 121 | 7 | 33 | the 7 are the 3 E0a files plus the 4 split Taskmgr files |
| `H/dist/i18n-snapshots/E-review/ui` (pre-review, 09:28) | 0 | 5 | 156 | `quicksettings`, `taskmgr/main`, `taskmgr/store`, `alles/main`, `disks/main` |
| `H/dist/win11shell-2026-11-04.tar.gz` (split 1+2+3a) | 157 | 4 | 0 | `taskmgr/{store,hostbanner,hostcall,pages/details}.js` |
| `H/dist/rollback/win11shell-live-2026-10-26-tree.tar.gz` | 111 | 39 | 11 | older than B |
| `H/ui` now | 117 | 44 | 0 | 161 files, no extras |
| `G/vendor/ui` now | 137 | 24 | 0 | plus 15 added files (176); `scripts/verify-ui.py` agrees: "bewuste wijzigingen: 24, toegevoegd: 15" |

So **B = the 2026-10-27 interim package**, which is also what Robin runs today. Between 10-27 and 11-04 H's UI changed
only in the 4 split files. Every later change is English work (F2).

### F2. H's 44 changed files, classified (step 2)

A change is assigned to a step by where the file stands in the snapshots (B, 11-04 tarball, E0a, E1, E-review, now)
and by its diff. Counts are lines added/removed from B to H.

**E0a only (3 files; English runtime):**
* `js/core/i18n.js` (+95/−7): `countOf` plural fix, `i18nReport`, `list()`, `collator()`; lines 74, 85, 101.
* `js/core/theme.js` (+3/−1): language in the theme cache.
* `js/mock/settings.js` (+3): the language enum.

**E1 only (33 files):**
* `core/bridge.js` (+75/−29): a `makeT` dictionary for errors, status and the launcher question. `X-Shell-Lang` at
  line 186. `e.key`/`e.params` on errors. `demoLauncherQuestion` as a dump hook.
* `core/format.js` (+20/−4): `Intl` caches.
* `desktop/desktop.js` (+11/−5): `collator`, host reload on a switch, `?open=launcher-trust`.
* `shell/search.js` (+42/−27): `qq_*` chips, `FALLBACK_SETTINGS` as `{nl,en}`, `homeFolder`, `collator`.
* `shell/toasts.js`, `shell/notifcenter.js`: "Melding" becomes `t('notification')`.
* `shell/startmenu.js`: `collator`, the search clear label.
* `shell/taskbar.js` (en "Task view").
* `settings/registry.js`: `collator` import.
* `settings/pages/{about,bluetooth,display,installed,startup,taskbar,time,update}.js`: collation, plurals, the endonym
  (D6), the export file name.
* `explorer/{main,util,i18n}.js`: tabs kept over the language reload (`saveForReload`/`restoreAfterReload`,
  H `explorer/main.js` lines 41–42, 1607, 1829, 1904); `debugOpen`; `TYPES_EN`.
* `alles/{actions,i18n,panes,table}.js`.
* `disks/{i18n,properties,util}.js`.
* `taskmgr/{actions,table,util}.js` and `taskmgr/storage/{cleanup,explorer,sunburst}.js`: plural objects and
  `collator`.

**E1 + E-review (4 files):**
* `alles/main.js` (+48/−4): state, sort and focus kept over the reload.
* `disks/main.js` (+27/−10): a load started in the old language is dropped.
* `taskmgr/main.js` (+27): `store.langChanged()`, `debugOpen`, restart-shell in mock mode only.
* `shell/quicksettings.js` (+15/−7): refresh after a switch; Wi-Fi `bad_args` keeps the password box open.

**Services split 3a only (1 file; not English):**
* `taskmgr/hostbanner.js` (+11/−3): the `downLive` message and `direct` routes.

**Split 3a + E1 (3 files):**
* `taskmgr/hostcall.js` (+147/−16): about 140 lines of split "direct routes", then E1's `X-Shell-Lang` (H line 117)
  and `key`/`params`.
* `taskmgr/pages/details.js`: split `notResponding` for background rows; E1 plural `many`.
* `taskmgr/store.js`: split `direct.live()`; E1 and E-review `inflightLang`/`gen` and `langChanged()`.

Other notes:
* E1 calls no API method and listens to no event that B does not have (method and event name diff: empty). On the UI
  side it is self-contained. Its host-side half (texts in the caller's language, `key`/`params`, notifications stored
  with `appName ""`) is H's E2/E3 work, which G's Rust host lacks.
* H/ui has not changed since 09:42. The rest of H is still moving: `notes/english-option-design.md` changed at 12:28,
  `i18n/host/display.tsv` at 10:41, `host/services/files.py` at 10:44 ("English round 2").

### F3. Codex's changes and their overlap with H (step 3)

`G/vendor/ui-patches.json` lists 24 changed and 15 added files, which matches the hash comparison.

**Overlapping (11), each with what G did and what H did:**

| File | G (B to G) | H (B to H) | Merge |
|---|---|---|---|
| `js/desktop/desktop.js` | +9/−2: "Deze PC" label and filter; `onLangChange` → `reloadSoon()` (G line 1117) | `collator`; `onLangChange` → `refreshView(); reload()` (H line 1116); launcher-trust hook | **1 conflict** (same line) |
| `js/apps/explorer/main.js` | +174/−23: clipboard gate, tab export/import, native window session (`saveWindowSession` G lines 378–398, `explorerSession` line 1948), picker | reload restore + `debugOpen` | clean, but two restore paths (see below) |
| `js/apps/alles/main.js` | +66/−13: live-updating overlay and status; **inline nl/en ternary** at G lines 421–423 | state over the reload | clean |
| `js/shell/search.js` | +32/−5: taskbar input integration; `onLangChange` rebuilds the settings index | chips, fallback pairs, `collator` | clean; both `onLangChange` parts kept (merged line 925) |
| `js/shell/startmenu.js` | +51/−7: software removal in menus, standalone entrance clip | `collator`, clear label | clean |
| `js/shell/taskbar.js` | +40/−11: real search input, symmetric centring; `onLangChange` resets `searchMode` (G lines 1051–1058) | en "Task view" | clean |
| `js/shell/toasts.js` | +10: replace-dedupe, GNOME `silent` | "Melding" → `t('notification')` | clean |
| `js/core/theme.js` | +48: native backdrop | E0a language cache | clean |
| `js/core/bridge.js` | +1: bundled settings icon | E1 (above) | clean |
| `js/apps/settings/registry.js` | +7/−3: power keywords | `collator` import | clean |
| `js/apps/settings/pages/installed.js` | +6/−7: native uninstall via `core/software.js` (removes keys `uninstall`/`uninstallDesc`) | `collator({sensitivity:'base'})` | clean |

**G-only, no H change (13):** `assets/wallpapers/bloom-{dark,light}.svg`, `css/{components,explorer,shell}.css`,
`js/apps/explorer/{detailspane,tabs}.js`, `js/apps/settings/pages/{background,power}.js`, `js/core/icons.js`,
`js/main.js`, `js/mock/system.js`, `js/shell/shell.js`.

**G-added (15), none touched by H:** `css/{defaultapps,installer,native-backdrop,picker,software}.css`,
`js/apps/{defaultapps,installer}/main.js`, `js/apps/explorer/{picker,tabtransfer}.js`, `js/core/software.js`, and
5 icon or wallpaper metadata files. Their language data is complete: `check.py` reports 0 nl/en gaps.

**Dutch-only literals that G introduced** (`scan_ui.mjs`, G minus B):
* `alles/main.js:421-423`: `getSetting('language','nl') === 'en' ? 'Indexing stops … open Everything again.' : 'Indexeren stopt …'`.
  This bypasses `makeT` and `?lang=`, and its English says "Everything" where D5 keeps "Alles".
* `explorer/main.js:368`: the rename-before-move error, as a `documentElement.lang === 'nl'` ternary.
* `explorer/tabtransfer.js:7`: a private `text(en, nl)` helper, used at lines 54 and 143 (2 texts).
* `explorer/picker.js:49`: `t('save')==='Opslaan'?'Opslaan als':'Save as'`. This is code that reads Dutch text (a §2.10
  coupling) and needs a `saveAs` key.
* `shell/shell.js:67`: `'startmenu'` is a module id. False positive; keep it.

**Host-side couplings the ported UI relies on, checked in G's Rust:**
* `services/notifications.rs:361` stores `appName: "Melding"` when the app name is empty. E1's UI shows its own word
  only for `""`, so G's English shows "Melding" until the Rust side gets the E3a change.
* `services/system/network.rs:336` returns `bad_args` / "Ongeldig wachtwoord.". This matches E1's code-based Wi-Fi
  check, so there is no regression.
* `services/mod.rs:62-68`: `language()` reads `settings.language` per call. No `X-Shell-Lang` is read anywhere in
  `host/src` (grep: 0 hits). G runs every app as a separate process with settings adopted from disk
  (`settings_runtime.rs:68`, README line 59). So H's D15 race (a refetch answered in the old language right after a
  switch) applies to G.

### F4. 3-way merge trial (step 4)

Command per file: `git merge-file -p -L G -L base -L H G/vendor/ui/F B/F H/ui/F` (current = G, base = B,
other = H). Output is in `G/notes/claude/map/merge-trial/`:

* `summary.tsv`: per file, which side changed it and the exit code (57 rows).
* `out/`: raw merge output for the 57 files changed on either side. `out/js/desktop/desktop.js` holds the conflict
  markers at lines 1118–1127. The H-only Taskmgr files in `out/` still carry split code.
* `out-nosplit/`: the 4 split-touched Taskmgr files, merged with `H/dist/win11shell-2026-11-04.tar.gz` as the base.
  The split hunks then count as "G removed" and drop out, and only E1's hunks are applied. All exit 0. For example,
  `hostcall.js` gains only the `lang` import, `makeError(…, from)` and `X-Shell-Lang` on its own `/api/call` fetch;
  `grep -c direct` = 0.
* `proposed/` and `proposed.tsv`: the candidate result, 43 files that differ from `G/vendor/ui`:
  * 32 files H-only;
  * 11 overlaps;
  * `hostbanner.js` stays as G has it;
  * the conflict resolved as
    `onLangChange(() => { if (loaded) { refreshView(); reloadSoon(); } });`. `reloadSoon` is the debounced
    `reload()` at line 328 of G's file.

Counts (176 files in total):

| Class | Files | Result |
|---|---|---|
| unchanged on both sides | 104 | clean |
| H only | 33 (of which 4 carry split code) | clean (`hostbanner.js`: keep G; 3 mixed: merge split-free, clean) |
| G only | 13 | clean |
| G added | 15 | clean (no H side) |
| both | 11 | 10 clean, **1 conflict** (`desktop.js`, 1 hunk, 5 vs 2 lines) |

Checks on the merged tree (scratch copy):
* `node --experimental-default-type=module --check`: 133/133 modules parse, so neither side added a duplicate import
  or declaration.
* `check.py --root <merged>`: 73 dictionaries, 2,674 keys, 162 inline pairs. 0 UI errors; 52 warnings, all
  `glossary`, the same 52 as H.
* `scan_ui.mjs`: 95 hits = H's 88 (all allowlisted `keep` in `H/tests/i18n/allow-ui.txt`, 84 rows) + the 7 from F3.

**Semantic overlaps that are textually clean but need a live test:**
* **Explorer** has two tab-restore paths. G's `pagehide` stores a native window session, and on a language-switch
  reload H's `sessionStorage` restore runs first and returns early (merged `start()`, `if (restoreAfterReload())
  return;`). The early return skips the `explorerSession`/`tabTicket` branch, which is fine on a reload. Untested:
  picker windows (`attachPicker`) and a tab transfer in progress during a switch.
* **Alles**: G's in-place overlay updates combined with H's reload that keeps its state.
* **Desktop**: G's debounced reload combined with H's immediate `refreshView()`.

### F5. Reusing H's tooling on G (step 5)

Tried: `scan_ui.mjs` and `check.py --root` ran unchanged against a scratch root whose `ui` symlinks to
`G/vendor/ui`, with `tests/i18n/dutch-words.txt` from H. Both worked; F3 and F4 come from those runs.

**Reusable after a path change**

Every tool assumes `ROOT/ui/js`. Sites to change:
* `check.py`: lines 359, 385, 391, 955
* `scan_ui.mjs`: lines 96, 103
* `corpus.py`: lines 82, 89
* `glossary.py`: lines 115, 129
* `audit.py`: lines 49, 218–219
* `ratchet.py`: line 278
* `intl_jsc.py`: line 27
* `serve.py`: line 45

The tools:
* `tools/i18n/jsdicts.mjs`, `tok.mjs`, `scan_ui.mjs`, `pairs.mjs`/`analyze.mjs` (the inventory tools). Pure JS
  parsing, nothing H-specific.
* `tools/i18n/check.py`: its TSV catalog rules (fields, placeholders, plurals, escapes, key = stem, glossary) and UI
  rules carry over. Its Python-host rules (`check_python`, line 708 and onward), `check_builtin` and `load_runtime`
  (line 123, needs `host/i18n.py`) and the bash rules (`SCRIPTS`, line 65) do not apply to G and need gating or
  replacing.
* `tools/i18n/ratchet.py`: the `ui` scan and its allowlist format carry over. `allow-ui.txt` keeps its 84 rows, plus
  7 new entries to convert or keep.
* `tools/i18n/glossary.tsv` (101 rows, Windows 11 en-GB, nothing Hyprland-specific apart from lock-screen terms) and
  `glossary.py`: as-is.
* `tests/i18n/dutch-words.txt` (1,295 lines): as-is. Optionally regenerate with `dutch_words.py` once G has catalogs.
* `tools/i18n/scan_en.py`: as-is (it reads an en dump).
* `tests/i18n/core_i18n.test.mjs`: as-is with the path change. `intl_jsc.py` also compares against
  `host/i18n.py join()` (line 74), which becomes the Rust `join` or is dropped.
* The UI dump (`tests/i18n/run.sh`, `serve.py`, `dump.html`, `freeze.js`, `blocks.py`, `surfaces.json`, 134
  surfaces, mock mode in headless Chrome):
  * The mechanics carry over: `serve.py` has to serve `vendor/ui` as `/ui/`.
  * `baseline-nl.txt` (18,123 lines) was recorded on H's tree and cannot be reused. G's Dutch output already differs
    (taskbar search input, power page, uninstall menu), so G needs its own baseline, recorded at `3b898e6` before the
    port.
  * `surfaces.json` needs G's new surfaces (Installer, Standaard-apps app, Explorer picker, software-removal menus,
    power page).
  * Those need mocks: G's UI calls 20 methods that B did not. Only the 2 `system.power.settings.*` methods are
    mocked, which leaves 18:
    `software.{cancel,execute,inspect,jobs,prepare,status,uninstallInfo}`,
    `apps.{forType,mimeTypes,setDefaultForType}`, `explorer.tab{Claim,Commit,Detach,Offer,Position,Release,Status}`,
    `nativeBackdrop.configure`.
  * The dump cannot reach code behind `window.__win11shellHost` (the Rust runtime branches).
* `tests/i18n/BASELINE-CHANGES`: the E0a and E1 UI rows carry over as G's intended Dutch changes. These are the
  "1.000 items" plural, the endonym "Nederlands (Nederland)", the singular fixes, "Afbeelding" and 17 new `?open=`
  surfaces. The E5, E6 and transcript rows do not.

**Catalogs:**
* `i18n/host/*.tsv`: 21 files, 588 rows. The TSV format (`key<TAB>nl<TAB>en`, `{name}`, `.one`/`.other`, fallback
  current → nl → key) fits Rust directly. G already uses this fill and fallback rule in `services/sysmon/common.rs`
  `tr()` (lines 191–200) and embeds tables with `include_str!`.
* Match of H's nl text with G's Rust sources (longest fragment, verbatim), by stem:
  * close to complete: disks 103/104, sysmon 98/104, storage 69/75, system 67/69, files 52/62, index 28/31,
    apps 25/28, tray 11/12, display 9/9, settings 8/8, shell 6/8, notifications 5/6, media 4/4,
    files_launch 4/4, filemanager1 1/1, clipboard 2/3;
  * Hyprland or Python-only: ctl 0/14, main 0/4, surfaces 1/11, windows 2/7, core 12/24.
  Reuse the en column; always take the nl text from G's Rust (Dutch must stay as G has it, not as H has it).
* G already has en for its `_STR` tables in JSON: `services/sysmon/text.json`, `services/disks/data.json`,
  `services/storage/data.json`, `services/system/strings.json`. Either `check.py` learns to read them, or they move
  to TSV.
* `i18n/{common,install,uninstall,session}.tsv` and `i18n/lib.sh` belong to H's bash installer and Hyprland session.
  They are not reusable: G's installer is `scripts/install.py` (142 lines) and `uninstall.py` (91), with about 10
  Dutch prints (`install.py:133-138`, `uninstall.py:91`) and a Dutch-only desktop `Comment` (`install.py:125-126`).
  `host/i18n.py`'s `detect_info()`/`dutch_signal()` (stdlib-only, D4/D13) can be reused directly by G's Python
  installer for the first-install choice.

**Not reusable (Python host or bash):**
* `host/i18n.py`: its design is ported to Rust, not its code (`L`/`Ln`/`pick`/`join`/`decimal`, request language,
  `settings_lang`).
* `tools/i18n/scan_py.py`, `extract_py.py` (1,438 lines), `scan_sh.py`, `extract_sh.py` (1,151 lines),
  `tests/i18n/transcripts.py` and `transcripts/`, `tests/i18n/nl-frozen/` (frozen nl of H's catalogs; G needs its own),
  `dist/i18n-e2` (conversion scripts).
* G needs a Rust scanner for the ratchet and either manual conversion or a small `extract_rs`.

## Work needed

Effort is in agent-hours, including tests and the step's own review.

| # | Item | Size |
|---|---|---|
| W1 | Apply `merge-trial/proposed/` (43 files) to `vendor/ui` on `claude/gnome-english-update`. Regenerate `vendor/ui-sha256.json`; update `ui-patches.json` (changed 24 → 56, reason text); `scripts/verify-ui.py` must pass, because `scripts/package.py:17` refuses otherwise. | 1 |
| W2 | Move the 6 G-introduced Dutch-only texts into `makeT` keys: Alles `firstNote` Rust variant (en with "Alles", via `t()` not `getSetting`), Explorer rename-before-move, the `tabtransfer.js` `text()` helper (2 texts), picker `saveAs` (removes the `=== 'Opslaan'` coupling). Allowlist `shell.js:67` as `keep`. | 1 |
| W3 | Present to Robin the Dutch changes that come with E0a/E1 (formatted-1000 plural, "Nederlands (Nederland)", 5 singular fixes, "Afbeelding"); record them in G's `BASELINE-CHANGES`. | 0.5 |
| W4 | Live test in a test GNOME session (not the live display): nl → en → nl with Explorer (tabs, tab transfer, picker open), Alles while indexing, Taakbeheer, desktop, quick settings Wi-Fi, Settings; Dutch unchanged otherwise. | 3 |
| W5 | Bring the UI tooling into G (`tools/i18n/`: `check.py`, `jsdicts.mjs`, `tok.mjs`, `scan_ui.mjs`, `ratchet.py`, `glossary.py`/`.tsv`, `corpus.py`, `dutch_words.py`, `scan_en.py`, UI part of `audit.py`; `tests/i18n/dutch-words.txt`, `allow-ui.txt`). One UI-root constant instead of about 15 hard-coded `ui/js` sites; gate the Python and bash rules; a test runner. | 3 |
| W6 | Rust carrier in the tools: `scan_rs` for the ratchet (literals with a Dutch word in `host/src/**/*.rs`, outside `#[cfg(test)]` and log macros; `ApiError::new` messages; the JSON `_STR` tables), `allow-host` for Rust, a key-exists check for Rust `L`/`tr` keys, and the TSV/JSON parity rules instead of `check_builtin`. | 5 |
| W7 | UI dump on G: port `run.sh`/`serve.py`/`dump.html`/`freeze.js`/`blocks.py`/`surfaces.json` (serve `vendor/ui`), mocks for the 18 G-only methods, new surfaces; record `baseline-nl.txt` at `3b898e6` **before** W1, compare after W1/W2. | 6–7 |
| W8 | `core_i18n.test.mjs` and `intl_jsc.py` on G (paths; `join` reference → Rust or dropped). | 1 |
| W9 | `host/src/i18n.rs`: parse TSV embedded with `include_str!` (so the catalogs cannot go missing at runtime); `L`/`Ln`/`pick`/`join`/`decimal`. Read `X-Shell-Lang` where `runtime.rs:253-257` reads `X-Shell-Token`, fall back to `services/mod.rs:62` `language()`, and pass it through `proxy()` (`runtime.rs:401`). Add `key`/`params` to the error envelope. | 5 |
| W10 | Catalog adoption: copy the 528 rows of the 16 H stems that exist in Rust (492 of them match G's Rust verbatim; 36 to reconcile, 12 of which are too short to test) and check each nl against G's literal. Drop or replace the ctl/main/surfaces/windows rows and the Python-`core` rows (60). Decide on the 4 JSON `_STR` tables. | 6 |
| W11 | Convert G's Dutch-only host texts: 203 literal Dutch `ApiError` messages plus other user-visible literals. About 274 Dutch literals have no H row: `process_scope.rs`, `picker.rs`, `transfers.rs`, `gnome.rs`, `backend.rs`, `window.rs`, software, defaults, portal…; part of them are logs and stay (D10). | 10–14 |
| W12 | Host couplings: `notifications.rs:361` "Melding" → `""` plus existing history; text in broadcast events and shared caches rendered per language; `index.status` re-sent after a switch. | 2 |
| W13 | Installer: `scripts/install.py`/`uninstall.py` prints (about 10), desktop entries `Name`/`Comment` with `[nl]`/`[en]` (`install.py:125-126`), first-install language through H's `detect_info()` (D4/D13). The extension has 1 bilingual site (`extension/notificationsBridge.js:214-220`); check it only. | 2–3 |
| W14 | `README.en.md` (README.md is 1,622 words) and the English review / glossary lock-in (H's E8). | 6 |

**Total: about 52–58 agent-hours.** UI port (W1–W4) about 5.5, tooling (W5–W8) about 15–16, host, installer and docs
(W9–W14) about 31–36.

**Order:** W7's baseline must be recorded before W1 changes any Dutch output. W5 can run in parallel with W7. W1 and
W2 come next, then W4. W9 must exist before W10–W12. A further Windows Update UI should be written bilingual from the
start (`makeT` nl+en) and pass W5's ratchet.

## Risks

* **H is not frozen.** H/ui has not changed since 09:42, but the notes (12:28), `i18n/host/display.tsv` (10:41) and
  `host/services/files.py` (10:44) are still changing. Fix a cut-off point for H. If H/ui changes again, re-run the
  trial (the procedure in F4, `summary.tsv`).
* **Taking H's files wholesale imports split code.** `taskmgr/hostcall.js`, `store.js`, `pages/details.js` and
  `hostbanner.js` would bring in the direct-worker routes. It stays inert while G's `shell.info` has no `workers`
  (hostcall's `known=false`), but it is dead code with a CORS assumption. Use `proposed/`, not `H/ui`.
* **"Dutch stays unchanged" versus E0a/E1.** E0a and E1 change some Dutch output on purpose (W3). Robin should accept
  this explicitly. If he does not, those hunks must be held back, and they sit in `i18n.js`, `time.js` and 6 plural
  sites.
* **The language-switch race (D15) applies to G.** Until W9 reads `X-Shell-Lang`, host texts can come back in the old
  language right after a switch, because every app is a separate Rust process that adopts settings from disk.
* **Coverage gap in the dump.** The dump harness sees mock mode only. G's Rust-only branches (`__win11shellHost`:
  Alles `firstNote`, Explorer session, native backdrop, standalone Start clip) and the real Rust texts need W4-style
  live checks.
* **Memory.** The dump needs about 1 GB and waits for 2 GB `MemAvailable`; about 3.4 GB was available today.
* **Look.** H changes no CSS or assets, so the Windows 11 look is not at risk from the UI port. G's own UI and CSS
  changes are untouched.

## Not checked

* No runtime test of any kind: no WebKitGTK, no Rust host, no cargo, no dump run, no nested GNOME session. "Clean"
  means textually clean, syntax-valid, checker-clean and scanner-clean, not behaviourally verified.
* Not every hunk of the 10 clean overlap merges was reviewed for semantics. The detailed reads were desktop,
  explorer, alles, search, taskbar, installed, toasts, bridge and theme. The H-only files were classified from
  snapshots and diffs, not reviewed line by line.
* The Rust counts (630 Dutch literals, 274 without an H row, 203 Dutch `ApiError` messages) are regex heuristics.
  They use H's `dutch-words.txt` and cut each file at its first `#[cfg(test)]`. Logs are not separated from
  user-visible text.
* Not read: G's `vendor/host` (the Python reference backend), Codex's `../../work/*` directories, G's
  `validation/` and `VALIDATION*.md`, the native GTK surfaces' own strings (for example `window.rs`), and G's
  desktop-entry generation beyond `install.py:125-126`.
* H's `tests/i18n/scratch/`, `dist/i18n-e2/`, the transcripts, and the E2–E6 snapshot contents (they have no `ui/`)
  were not inspected.
* The live session, `~/.config/*` and the extensions directory were not touched. `~/.local/share/win11shell/ui` was
  read for hashes and diffs only.
