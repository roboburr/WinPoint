# GE0a: UI i18n tooling and the Dutch/English UI baseline (2026-11-05)

G = this repo, branch `claude/gnome-english-update` (HEAD `7fc86a6d`; the UI is untouched since `3b898e6`).
H = the frozen Hyprland project. Nothing was committed. No product code was changed (`host/src`, `vendor/ui` outside
`js/mock/`, `extension`, `scripts`). No Win11 binary, gnome-shell, gsettings, dconf or installer ran.

## 1. Result

| | |
|---|---|
| Surfaces | **139**: H's 117 E0b surfaces + 22 G-only. Per area: shell 27, desktop 8, settings 37, taskmgr 8, explorer 22, alles 12, disks 11, defaultapps 4, installer 10. 8 of them use harness actions (`do`). |
| `tests/i18n/baseline-nl.txt` | 15,956 lines, 361,480 bytes, sha256 `5db4b8f9b5c79303594ea10b06d4314419c407475f810fa25d9a0138e4c91fb4`, Google Chrome 156.0.8078.73 |
| `tests/i18n/baseline-en.txt` | 15,952 lines, 339,861 bytes, sha256 `912ee8b75043deab47c2b7c3a1daa2eefa07ca739dc3a2ffa9747e814a579547` |
| Determinism | Each language was dumped 4 times: the record run, `--twice` (2 dumps) and a compare. All 4 were byte-identical. A copy of the tree with one extra first line in `core/bridge.js` and `core/theme.js` also reproduced both baselines, so the seeded mock numbers do not depend on line numbers. Log: `notes/claude/round0/ge0a-dump-runs.log`. One dump of 139 surfaces takes 12–13 s. |
| Harness trouble | none: no `NOT READY`, `NOT IDLE`, `HARNESS ERROR`, `DO FAILED`, "Onbekende functie", console error or warning, or uncaught error on any of the 139 surfaces (`run.sh --notes`), in nl or en. |
| `tools/i18n/check.py` | **0 errors, 69 warnings**: glossary 54 (52 UI + 2 in `disks/data.json`), plural-noun 14, table-gap 1. Stats: 72 `makeT` dictionaries, 2,638 keys, 117 inline/P() pairs, 3 X_NL/X_EN tables, 134 JS files, 250 rows in the 4 Rust JSON tables, 0 TSV files. |
| `tools/i18n/ratchet.py` | ok: 157 occurrences in 18 files, 150 entries in `allow-ui.txt`; 90 `keep` (84 carried over from H's statuses, all matched), 67 still to convert |
| `tests/i18n/test_ge0a.py` | 21 tests: 19 ok, 2 heavy ones skipped by default (with `W11_I18N_HEAVY=1` they repeat the compares above) |

## 2. What was built

**`tests/i18n/`**

| File | What |
|---|---|
| `run.sh` | Ported from H. It serves the tree, runs `dump.html` in headless Chrome and records, compares or does `--twice`. Changes from H: `--lang en` compares with `baseline-en.txt`, and `--record --lang en` records it (H recorded nl only). New `--notes FILE` writes a JSON of console messages per surface. Chrome runs only through `private.sh` under `flock -w 3600 $G/.heavy.lock`, after a check for at least 2 GB `MemAvailable`; a memory drop while it waits for the lock means a retry. Chrome flags: no network (`--host-resolver-rules` maps everything but 127.0.0.1 to NOTFOUND), no component update, no breakpad. `HEAVY_LOCK`, `CHROME` and `BUDGET` can be overridden. |
| `private.sh` | **New** (not in H). It runs one command in a private session. `env -i` keeps nothing of the live session, so there is no DISPLAY, WAYLAND_DISPLAY or DBUS address. HOME, XDG_CONFIG/DATA/CACHE_HOME, XDG_RUNTIME_DIR (0700) and TMPDIR all live in a work dir, which must be inside `G/tests/`. It sets `GSETTINGS_BACKEND=keyfile`, `NO_AT_BRIDGE=1` and `GTK_A11Y=none`, and `dbus-run-session` gets a generated config with an **abstract socket** (nothing in /tmp) and **no service directories**, so nothing can be activated on that bus: no at-spi launcher (memory note "Private D-Bus clobbers a11y bus"), no portal, no gvfs. Guards run inside the session and exit 70 without running the command when they fail. Chrome's singleton socket would be too long for `sun_path` inside the repo, so `run.sh` gives Chrome a **relative** `TMPDIR` (`tmp`, inside the work dir) instead of H's `/tmp`. |
| `serve.py` | Ported from H. It serves only `/ui/` → `vendor/ui/` and `/tests/i18n/` → `tests/i18n/`; everything else is 404. It is read-only (POST/PUT/DELETE give 405), does not list directories, sends `no-store`, and puts `freeze.js` first in `/ui/index.html`. The UI keeps H's `/ui/js/…` paths, so the URLs and the freeze.js random streams look the same as under the Rust host. |
| `dump.html` | Ported from H. Three G changes. (1) **Harness actions**: a surface may list `do: [{click\|contextmenu: selector, nth?, wait?}]`, which runs after the page is idle at its stop time; the stop time then moves on (`freeze.js extend`), and a `# do: …` line goes into the block. This is for G surfaces without an `?open=` hook. (2) G's `core/i18n.js` has no `?i18ncheck=1`, so H's per-block `# NO i18n REPORT` line went to `#notes` instead. The nl baseline therefore stays comparable after GE1a ports H's i18n.js; a `# i18n missing/unfilled/plural` line still appears in a block whenever the report has entries. (3) Per-surface console messages go into `#notes`. |
| `freeze.js` | H's file plus two harness-only additions. `__i18nFreeze.extend(ms)` re-arms held timers with their own delay, so Chrome's virtual time still waits for pending fetches. `__i18nFreeze.console()` collects console.error/warn, uncaught errors and unhandled rejections (message text only). The clock, timer order and random streams are unchanged. |
| `blocks.py`, `dutch-words.txt` | H's files, unchanged (`dutch-words.txt`: 1,291 words) |
| `surfaces.json` | H's 134 surfaces minus the 17 that need H's E1 `?open=` hooks (GE1a adds those as new blocks), plus 22 G-only surfaces: Start and all-apps context menus with "Verwijderen", Settings › Installed apps menu and uninstall review, Explorer with two tabs, the picker (open, save, folder), Standaard-apps (list, search, choose dialog, a type without apps) and the App-installatieprogramma (empty, activity, deb, AppImage, snap, unsupported file, uninstall, protected uninstall, review dialog, finished dry-run progress) |
| `allow-ui.txt` | Generated from G (`ratchet.py --write --yes ui`). H's 84 `keep` statuses were carried over, after the path change, where file and hash matched; all 84 matched. |
| `MOCK-CHANGES.md` | every mock change, with its reason and its effect on the dump |
| `BASELINE-CHANGES` | the header and format only; no Dutch change so far |
| `test_ge0a.py` | Unit tests. Ratchet ok, allowlist well-formed, scanner rules and counts/stale on fixtures, `--allow` needs a reason, check.py has 0 errors. Every UI-called method is mocked and every mock file is loaded. surfaces.json matches the Settings registry, Taakbeheer's tabs and `main.js` APPS. Both baselines are complete dumps of surfaces.json with no trouble markers, and BASELINE-CHANGES rows are well-formed. The server only serves its mounts and is read-only. `private.sh` refuses work dirs outside `tests/` without creating them and hands a clean environment. Heavy, opt-in: both compares. |

**`tools/i18n/`** (re-rooted to `vendor/ui/js` through `paths.py`: `UI_REL`, `HOST_JSON_TABLES`)

| File | From H |
|---|---|
| `tok.mjs`, `jsdicts.mjs`, `pairs.mjs`, `analyze.mjs`, `glossary.tsv` | unchanged (`jsdicts.mjs` and the two inventory tools take the UI dir as an argument) |
| `scan_ui.mjs` | `UI_REL = 'vendor/ui/js'`; its hits are named `vendor/ui/js/…` |
| `paths.py` | **New.** The one place for G's paths. |
| `check.py` | Kept: the TSV rules (fields, key and stem, duplicates, empty, escapes, placeholders, plurals, article, glossary), all UI rules and the README.md/README.en.md shape. New: parity and placeholder checks of the 4 Rust JSON `{nl,en}` tables (`str-table`, a warning until GE1b). `DEFERRED` now names G steps (GE1a, GE1b, round 3). Dropped: Python AST, `host/i18n.py` built-ins, bash keys and lib.sh, `{b}`/`{dim}` styling, `#!shell-quoted`, hyprlock markup, the `\n` rule, desktop `[nl]`/`[en]` (D-G4), install.sh help blocks. |
| `ratchet.py` | UI scan only (H's `host`/`sh` scans are Python and bash). `PREFIX` maps `--allow FILE:HASH` to a scan. A Rust scan is one more entry in ALLOW, PREFIX, WHAT and `scan()`. |
| `corpus.py`, `glossary.py`, `dutch_words.py` | Re-rooted. H's Python `_STR` and `{"nl","en"}` readers were replaced by readers of the 4 Rust JSON tables (`corpus.host_json_tables`). |
| `scan_en.py` | Re-rooted. It reads a dump: `python3 tools/i18n/scan_en.py tests/i18n/baseline-en.txt`. |

**`vendor/ui/js/mock/`** (see `tests/i18n/MOCK-CHANGES.md`): new `gnome.js` with the 18 G-only methods; `index.js` loads it and warns on an
unknown method.

## 3. How H's harness works (and what was not Hyprland-specific)

H rendered with **headless Google Chrome**, not WebKit and not nested Hyprland: `--headless=new --dump-dom` with
`--virtual-time-budget`, a throwaway profile, and a static server on 127.0.0.1. Nothing in it depended on Hyprland,
so the mechanism carries over unchanged. Only the isolation is new (`private.sh`). Chrome reports
`HeadlessChrome/156.0.0.0` and uses the TZ Europe/Amsterdam that `private.sh` sets.

## 4. Commands to reproduce (from G)

```bash
export PATH=/home/robin/.local/node/bin:$PATH          # node 20 for the UI tools (corpus.find_node also finds it)
tests/i18n/run.sh                      # nl dump == baseline-nl.txt (exit 1 and a diff otherwise)
tests/i18n/run.sh --lang en            # en dump == baseline-en.txt
tests/i18n/run.sh --twice              # two nl dumps identical (add --lang en for English)
tests/i18n/run.sh --out FILE --notes NOTES.json        # a dump plus its console notes, no compare
tests/i18n/run.sh --record [--lang en] [--accept ID,…] # re-record; a changed block needs --accept + BASELINE-CHANGES
python3 -B tools/i18n/check.py [--warnings]
python3 -B tools/i18n/ratchet.py [--summary]
python3 -B tools/i18n/glossary.py | head; python3 -B tools/i18n/scan_en.py tests/i18n/baseline-en.txt
python3 -B -m unittest tests/i18n/test_ge0a.py -v      # W11_I18N_HEAVY=1 adds the two compares
```

The 4-run determinism sequence: `run.sh --record`, `run.sh --record --lang en`, then `run.sh --twice`,
`run.sh --twice --lang en`, `run.sh` and `run.sh --lang en`.

## 5. Numbers worth knowing

* **Mock effect.** A dump without `gnome.js` differed from the baseline only in the 18 surfaces that call the new
  methods: each showed "Onbekende functie: software.uninstallInfo / apps.mimeTypes / software.inspect …". The other
  121 blocks were byte-identical with and without the new mock.
* **check.py warnings.** Glossary: 52 UI + 2 in `host/src/services/disks/data.json`. Plural-noun (14): taskmgr
  actions 3, disks 3, explorer 2, alles 2, sunburst, details, display, about. Table-gap: `explorer/util.js` 1. These
  agree with the map: H counted 52 glossary warnings for the merged tree.
* **Ratchet.** 157 hits = the count ui-apps reported for G.
  * Still to convert (67): search.js 35, bridge.js 15, alles/panes.js 5 (the tip examples).
  * Codex's 6 texts: alles/main.js "Indexeren stopt …", explorer/main.js "Rond het hernoemen eerst af …",
    picker.js "Opslaan" and "Opslaan als", and 2 in tabtransfer.js.
  * 6 base texts that H's tree no longer lists: alles/main.js `'dm:vandaag'`, explorer/util.js `${…}-bestand` and
    "Snelkoppeling", alles/i18n.js "Afbeelding", the update.js file name, startmenu.js `'Wissen'`.
  * Keep: shell.js's `'startmenu'` module id (×2) has H's keep reason.
* **The English dump** has 1,431 lines with a Dutch word on 100 of 139 surfaces (`scan_en.py`), almost all of it mock
  data and mock host texts. The mocks answer in `settings.language`, which stays nl when only `?lang=en` is set.
  This is H's known state; allow-en.txt comes with round 3.
* **`dutch_words.py`.** A regeneration from G would add 45 words and drop 4. The added words come from Codex's UI
  (installatie, pakketbeheerder, energiebesparing, sluimerstand, voltooid …); the dropped ones are aanvraag, functie,
  moest and proef. The committed list stays H's (port-path: "as-is"), so the ratchet can miss a new Dutch-only
  literal that uses only these 45 words.

## 6. Limitations

* **The engine is Chrome, the product is WebKitGTK.** The baseline is a Chrome text dump. Intl output (dates,
  `108,41 MB`, lists) may differ in WebKitGTK. `run.sh` warns when the Chrome version changes.
* **Mock mode only.** Branches behind `window.__win11shellHost` cannot be reached: the Rust runtime, the Alles
  `firstNoteNative`, the Explorer session restore, the native backdrop and the standalone Start clip. The Rust host
  texts are not dumped either. That is the job of GE0b (Rust literals) and GE0c (private GNOME screenshots).
* **Not reached without a hook.** Codex's Explorer rename-before-move error (`explorer/main.js:368`), the
  `tabtransfer.js` texts (they need a drag between two windows), the Verkenner, Taakbeheer and Schijven
  menus and dialogs (H's E1 hooks, GE1a), and every error toast that needs a failing call.
* **The actions are a G addition.** A `do` step is deterministic as long as the selector picks the same element.
  A UI change that reorders rows can move it; the block then shows another app, or a `# DO FAILED` line that the
  test catches.
* **No i18n report yet.** `window.__i18n` (missing keys, unfilled placeholders) does not exist at 3b898e6. It
  arrives with GE1a's `core/i18n.js`. Report lines will then appear only in blocks that have entries.
* **The UI manifest is broken by the mock changes.** `python3 scripts/verify-ui.py` exits 1: `js/mock/gnome.js` is
  extra and `js/mock/index.js` changed. So `package.py` refuses to package until `vendor/ui-sha256.json` and
  `ui-patches.json` are regenerated. That regeneration is GE1a's job (gnome-map §2.5 step 4).

## 7. Not done (on purpose or out of scope)

* **Not ported:**
  * H's Python-host and bash tools: `scan_py.py`, `scan_sh.py`, `extract_py.py`, `extract_sh.py`,
    `transcripts.py` and its transcripts, `nl-frozen/`, `allow-host.txt`, `allow-sh.txt`.
  * `audit.py` and `audit.tsv`, H's §2.10 audit. Its UI part could be ported later: the "code that reads Dutch text"
    hits, such as picker.js's `t('save') === 'Opslaan'`.
  * `core_i18n.test.mjs` and `intl_jsc.py`. They test H's E0a API (`countOf`, `list`, `collator`), which G's
    `core/i18n.js` does not have yet; they belong with GE1a.
* **Not done:** no English review, no change to `dutch-words.txt`, no `allow-en.txt`, no screenshots.
* **Not checked:** the live session and `~/.config/*` were not touched.

## 8. For the coordinator to decide

1. **The UI manifest.** Accept that `scripts/verify-ui.py` fails until GE1a regenerates `vendor/ui-sha256.json` and
   lists the mock changes in `ui-patches.json`; or exclude `js/mock/` from the manifest. Either way, nothing gets
   packaged before then.
2. **`dutch-words.txt`.** Keep H's list (now), or regenerate it from G with `dutch_words.py --write` as a reviewed
   step. A regeneration adds 45 words from Codex's UI, drops 4, and may add ratchet hits.
3. **The H E1 surfaces.** GE1a should copy H's 17 E1 surfaces into `surfaces.json` and record them as new blocks
   with `run.sh --record`, which needs no `--accept`. The explorer.* ids keep H's names. The other blocks must then
   stay identical, apart from the BASELINE-CHANGES rows for D-G1.
4. **The en baseline.** It will change with every English step. Re-record it per step with `--accept`, and keep a
   BASELINE-CHANGES line only for intended *Dutch* changes. Rule needed: is an en block change a "reviewed" change
   that also needs a line?
5. **Ownership of `tools/i18n/`.** GE0b added `tools/i18n/scan_rust.py` next to these tools. It reads
   `tests/i18n/dutch-words.txt` and does not use `paths.py`. Wiring a `rs` scan into `ratchet.py` would be one entry,
   as described in its docstring.
