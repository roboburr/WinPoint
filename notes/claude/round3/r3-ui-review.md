# R3-UI adversarial review (2026-11-06)

Reviewed in the worktree `.wt/r3-ui` (branch `r3/r3-ui`, HEAD 18c8ffbe, nothing committed). I edited no product file.
I wrote only this note and scratch files under `.rt/review-r3-ui/`: the HEAD tree from `git archive`, a mutable copy
for mutation tests, dumps, strace logs, screenshots and sheets. Every heavy command ran through `tools/heavy.sh` with
a tag of `r3-ui-rv-*`. `shoot.sh` writes into `tests/update-ui/shots/`, which did not exist before my run. I moved its
PNGs to `.rt/review-r3-ui/shots/` and removed the directory. heavy.sh wrote its usual logs to `tests/logs/`, which git
ignores.

**Verdict: pass.** I found no blocker and no major. Six minor findings need fixing: two are English defects the step
introduced, and two are English defects of the same kind the review missed. There are also eight notes.

## 1. Done-when, run by me (from W's root, `PATH=/home/robin/.local/node/bin:$PATH`)

| Check | Result |
|---|---|
| `python3 -B tools/i18n/check.py` | rc 0: `0 error(s), 72 warning(s) (rust 72)`, `glossary_context_rows=18`. At HEAD (archive): `0 error(s), 124 warning(s) (glossary 52 …, rust 72)` |
| Glossary as an error, by mutation (`.rt/review-r3-ui/mut`, `check.py --root`) | en "Goes to the Trash" → `error glossary … cleanup.js:21`, rc 1. update.js `retry: 'Try again'` → error (Retry row), rc 1. common.js `retry: 'Retry'` → error, rc 1. An extension pair "Win11 Preferences" → `error pairs … glossary`, rc 1. `healthDesc` nl reworded → `error glossary-context … glossary-context.tsv:21 … (stale row)` |
| `python3 -B tools/i18n/ratchet.py` | `ui: ok (89 allowed occurrences, 0 still to convert)` |
| `check_ui.py --dict-diff tests/i18n/dict-nl-3b898e6.tsv --quiet` | `2925 → 3170 rows; 9 changed, 20 removed, 265 added; 0 not in BASELINE-CHANGES.ui, 0 stale` (HEAD: 3164 rows, 259 added) |
| nl dictionary snapshot, HEAD vs now (`--dict-snapshot`, diff) | only 6 added rows: `advNotifyDesc`, `advSnapsHeld`, `advSnapsHeldForever`, `advMore`, `hRecovery`, `hRecoveryDesc`; no nl value changed |
| `tools/heavy.sh --tag r3-ui-rv-dump -- tests/i18n/run.sh` | `nl dump equals baseline-nl.txt (24786 lines)`, rc 0 |
| `… run.sh --lang en` | `en dump equals baseline-en.txt (24783 lines)`, rc 0; the console note is now English ("[offline] No connection to Win11 Shell. Try again in a moment.") |
| `tools/heavy.sh --tag r3-ui-rv-twice -- tests/i18n/run.sh --twice` (nl, en) | `two nl dumps identical (24786 lines, sha256 9231043098614713)`, `two en dumps identical (24783 lines, sha256 ca71d5699faa149b)`: the same hashes as the step's |
| Block diff of the baselines against HEAD (`.rt/review-r3-ui/blockdiff.py`) | nl: 16 changed blocks, 2 new (S03j, S03k), 0 gone. Every change is a pure addition of the 5 D-G44 lines, with no line removed or moved. en: 43 changed blocks, 2 new. Both lists equal the step's `--accept` lists (`.rt/r3-ui/accept-{nl,en}.txt`) exactly |
| `node --input-type=module --check` | the 49 changed modules, `jsdicts.mjs` and `mock_settings.test.mjs`: 51 checked, 0 failed |
| `node tests/i18n/core_i18n.test.mjs` / `node --experimental-default-type=module tests/i18n/mock_settings.test.mjs` | `ok 24214` / `ok 16` |
| `python3 -B -m unittest` on `tests/i18n/test_*.py` | test_ge0a 21 OK (2 skipped); test_glossary_context 8 OK; test_update_mock_rows 5 OK (1 skipped) |
| `TZ=Europe/Amsterdam python3 -B tests/i18n/intl_jsc.py`, `check_pairs.py`, `glossary.py` | 66 values, 0 mismatches; `check_pairs: 0 errors, 0 warnings (… pairs 144 …)`; the report lists 18 context rows (1 known) |
| `python3 -B tools/ui-manifest.py --check`, `python3 -B scripts/verify-ui.py` | `up to date (vendor/ui: 181 files; … 82 changed, 20 added, 0 removed)`; `OK: 181 interfacebestanden …` |
| `check_ui.py`, `check.py --no-js`, `check.py --layers ui` | no crash; 0 errors |
| Screenshots: `tools/heavy.sh --tag r3-ui-rv-shots -- tests/update-ui/shoot.sh <19 ids>`, and `LANGS=en SIZE=800x1400 SUFFIX=-narrow … <6 ids>` | `38 shots, 0 failed`; `6 shots, 0 failed` (§4) |

## 2. Safety (CHECK 1)

* **The step's tests make no socket calls.** I ran `mock_settings.test.mjs`, `test_glossary_context.py`, `check.py`
  and `core_i18n.test.mjs` under `strace -f -e trace=connect,socket,sendto,bind`. Not one socket call was made
  (`.rt/review-r3-ui/strace/*.txt`). test_glossary_context's temporary files go into a `TemporaryDirectory` that is
  deleted at once.
* **A dump with the new surfaces stays inside the layer.** Command: `tools/heavy.sh --tag r3-ui-rv-strace -- strace -f
  -e trace=connect,sendto … tests/i18n/run.sh --lang en --only
  settings-update-advanced.S03j,settings.update-history,taskmgr-dialog.restart-shell --out …`, rc 0.
  * 10,337 trace lines. The only successful or in-progress connects go to 127.0.0.1 (the local server) and to
    private.sh's abstract `@w11-i18n-…` bus.
  * Every other attempt failed. `/run/dbus/system_bus_socket` (6 attempts) gave ENOENT. Chrome's own probe to
    `2001:4860:4860::8888:443` gave ENETUNREACH. `/dev/log`, nscd and sss gave ENOENT.
  * nonet.sh's layer holds. Chrome still makes these attempts on its own; that is not new and is not R3-UI's.
* **No new path to the host from the code.** The new code paths are `ctx.go`, `apps.list` and `scrollIntoView`. They
  add no process, no file write and no host call apart from the existing `apps.list`. `mock/index.js` and
  `mock/shell.js` import only `core/i18n.js`.

## 3. Conformance (CHECK 2)

| Requirement | Status |
|---|---|
| 1 English review, widths | Done, except m1-m4 (§5). Widths: en dropdowns are 240 px, nl stays 220 px (`display.js` `ddWidth`), and the shots show no clipping |
| 2 52 warnings, glossary as an error, Retry/Try again | Done: 0 glossary findings; DEFERRED no longer has `glossary`; check_pairs reports errors; mutations fail as intended. "Retry" for `update*.js`, "Try again" in glossary.tsv. Host TSVs are not locked in (m5) |
| 3 cross.md item 6 | Done: `time.js` `langValue()`, the C1 rule |
| 3 cross.md item 8 | Done: `preview/wm.js` `alles: 'Alles'` |
| 3 D-G31 mock | Done. Checked against `settings.rs` set/patch: same codes and data, wrong types first in a patch. Import keeps the current language (n4) |
| 3 D-G41 mock/index.js key/params | Done. Verified with `.rt/review-r3-ui/mock_index_check.mjs`: `settings.set` with "fr" gives `{code:'invalid_value', key:'settings.badValue', params:{}}`, and an unknown method in en gives "Unknown function: no.such". `data` is still dropped (n2) |
| 3 MOCK-CHANGES rows (mock/update.js, `rebootRequired`, update defaults, `shell.restartAll` incl. pending) | Done |
| 3 Start tiles at once | Code done: `shell.js` `onLangChange(load)` plus a sequence guard. The host's `apps.list` really answers in the current language: `apps/mod.rs` `language_in`, and `ensure()` rebuilds on a mismatch. Not verified in GNOME (m6) |
| 3 mock offline text follows the language | Done (en dump note is English) |
| 4 D-G44 held refreshes, two states and surfaces | Done. `next` semantics match `options.rs:191-199`: the end of the hold, null for forever |
| 4 "Aanvullende opties", restart-row description, "Gerelateerde instellingen"/Herstel, 6 keys in BASELINE-ADDITIONS.ui | Done (n3 on the wording) |
| 4 restart-all button stays disabled after an ok answer | Code done (`restartAllSent`, `onOk` before the repaint; `holding()` disables and the repaint keeps it disabled). Untestable in the mock (m6) |
| 4 MOCK-CHANGES row GX1 n9 | Done |
| 5 baselines only through `--record --accept`, every changed block listed | Done. The accept lists equal the changed blocks; BASELINE-CHANGES.ui has the 16 nl ids with D-G44 |
| Note `r3-ui.md` | Present. Inaccuracy: §4.5 says check_rust has 3 glossary warnings; it has 4 (m5) |

**Dutch (CHECK 4).** Every nl block change is a D-G44 addition, and no existing nl dictionary value changed (dict
snapshot diff). The mock texts are byte-identical in nl ("Onbekende functie: …", "Er ging iets mis.", "Deze waarde
past niet bij deze instelling.", the offline text). The "for Robin" items are unchanged: "Nederlands", Afbeelding and
the Alles hints, and "1 bestanden, 1 mappen". The only new error matching in the code is on `code`
(`unknown_method`); none matches on message text.

**Ownership (CHECK 6).** `git status --short` and `git diff --stat` show 64 modified and 4 new paths. All are inside
the allowlist:
* `vendor/ui/**`, with neither `taskmgr/pages/settings.js` nor `update-release.js` touched;
* `tests/i18n/**`;
* `tools/i18n/**` other than `check_rust.py`, which is untouched;
* `notes/claude/round3/r3-ui.md`.

Both `mock/update.js` hunks (the scenario comment at :449 and the `S03j`/`S03k` OPTIONS at :520-522) are inside the
GW4 section, which runs from :440 to before :623.

## 4. Look (CHECK 5)

I looked at these nl and en shots at 1280×1400:
* Settings: `settings.display`, `.network`, `.power`, `.privacy`, `.update-advanced`, `.update-history`;
  `settings-update-advanced.S03j`/`.S03k`.
* Shell and desktop: `shell.quick`, `desktop.emptytrash`, `desktop.properties`.
* Task Manager: `taskmgr-dialog.end`/`.restart`/`.properties`, `taskmgr.services`, `taskmgr.summary`.
* Explorer, Disks, Alles: `explorer.properties.multi`, `disks.initialize`, `alles.properties`.

At 800×1400 in en: S03j, `update-history`, `display`, `network`, `update-advanced`, `taskmgr-dialog.restart`. Copies and
nl/en sheets are in `.rt/review-r3-ui/shots/`.

* **Geavanceerde opties.** "Additional options"/"Aanvullende opties" is a group header above the three icon rows, as
  in 24H2. The restart-row description is one line at 1280 px and wraps cleanly at 800 px. The held text is one line
  at 1280 px and wraps after "7 hours ago" at 800 px.
* **Updategeschiedenis.** "Related settings"/"Gerelateerde instellingen" with a Recovery/Herstel card and a chevron.
* **Display.** "2560 × 1440 (Recommended)" fits the 240 px dropdown at both widths.
* **Dialogs and the Flight mode tile.** The longer English sentences wrap inside the dialogs, and nothing is cut.
* The content bounding boxes are the same in nl and en for every surface. No overflow.

## 5. Findings

### m1 (minor, introduced): the same Task Manager text has two English renderings now

* Evidence:
  * `apps/taskmgr/actions.js:80` became "System process – can’t be ended".
  * `pages/details.js:29` and `pages/processes.js:36` still say "System process – cannot be ended".
  * All three have the same nl text, "Systeemproces – kan niet worden beëindigd". The context menu and the End task
    button's tooltip now disagree.
  * Reproduce with `.rt/review-r3-ui/pairs.py` (nl texts with more than one en rendering): this is the only new one
    against HEAD.
* Fix: use "can’t be ended" in `details.js:29` and `processes.js:36`.

### m2 (minor, introduced): "Nothing of the kind" is an idiom for "certainly not"

* Evidence: `apps/alles/i18n.js:183` changed "Nothing of kind “{f}” matches “{q}”." to "Nothing of the kind “{f}”
  matches “{q}”." {f} is a kind such as "Picture" or "Folder". "Nothing of the kind" reads as a denial.
* Fix: "No items of type “{f}” match “{q}”." (or "No “{f}” items match “{q}”.").

### m3 (minor): Dutch-ordered or awkward English that the review left

Each of these is visible to the user or is a sibling of a text the step did edit:
* `taskmgr/actions.js:70-71` `endBodyMany`: "This closes {n} processes of {name}." (nl "Dit sluit … processen van
  …"). It shows in the en dump on `taskmgr-dialog.end`: "This closes 17 processes of Firefox." The step rewrote only
  its second sentence. Fix: "All {n} processes of {name} will close." or "{name} and its {n} processes will close."
  (`one`: "{name} will close.").
* `core/software.js:34` `retryReview`: "Check Activity before you try again. To try again, review the operation once
  more." This repeats "try again". Fix: "Check Activity before you try again, and review the operation once more."
* `taskmgr/storage/explorer.js:32` `hostDown`: "… Scan again once it’s working again." Fix: "… Scan again once it’s
  back."
* `alles/i18n.js` `opt_rebuildDesc`: "Builds the overview again from scratch." Fix: "Rebuilds the overview from
  scratch." Also `queryError`: "This search isn’t quite right", a calque of "klopt niet helemaal". Fix: "There’s a
  problem with this search".

### m4 (minor): the consistency pass stopped short

The step aligned the desktop's "Text Document" with Verkenner's, but Alles keeps the old forms:
* `alles/i18n.js:203,205` say "{ext} file" and "Text document". Verkenner says "{ext} File"
  (`explorer/i18n.js:250`) and "Text Document" (`:170`), Windows' type names.
* The same Empty Recycle Bin message has two renderings:
  * Verkenner (`explorer/i18n.js:229-230`) and search (`shell/search.js:77`): "… will be deleted for good. This
    can’t be undone."
  * The desktop (`desktop/strings.js:182`): "… will be permanently deleted. You can’t undo this."

Fix: "{ext} File" and "Text Document" in Alles, and one rendering for the Recycle Bin message, preferably the
desktop's "permanently deleted".

### m5 (minor, coordination): host catalogues are not locked in, and the note miscounts

* Evidence: `check.py --warnings` still prints 4 "[glossary]" findings under rule `rust`:
  * `disks.unavailableBody`;
  * `sysmon.restart.noEnviron`;
  * `update.err.checkFailed`;
  * `update.part.pixbuf`.
* The same 4 appear at HEAD. r3-ui.md §1/§4.5 says "3"; `disks.unavailableBody` is missing from its list.
* check_rust replaces the shared `tsv` rule, so a host TSV row can still miss a glossary phrase without failing
  check.py. That is R3-NATIVE's file, so it is not R3-UI's defect, but "0 glossary warnings" holds only for the rule
  named `glossary`.
* Fix:
  * Correct the count in r3-ui.md.
  * Coordinator: R3-NATIVE makes check_rust's glossary rule an error and reads `glossary-context.tsv` for host rows
    (or adds context rows for the 4). Until then the lock-in covers the UI, the pairs and the host JSON tables only.

### m6 (minor, test gap): three behaviour changes have no test or surface

* **The restart-all button.** The disabled state cannot be reached in mock mode: `mock/shell.js` always answers
  `dryRun: true` (`return { ok: true, nonce, dryRun: true }`), and `restartAllSent` is set only when the answer is
  not a dry run. No unit test covers it.
* **The Start tiles.** The immediate reload after a language change is not measured anywhere. The step skipped the
  harness, and the mock's `apps.list` is Dutch only.
* **`time.js` `langValue()` and the Recovery deep link.** Neither is on a dump surface.
* Fix:
  * The coordinator runs the step's harness command (r3-ui.md §8 item 5) on the merged build. The command writes into
    `tests/gnome/`, which is why I did not run it.
  * Optionally a mock switch so `shell.restartAll` can answer non-dry, plus a surface or node test for the disabled
    button.
  * A surface with a hand-edited `language` ("EN") for `langValue()`.

### Notes

* **n1, the Recovery deep link scrolls again after a re-render.**
  * `update.js:852-854` scrolls the backup group into view in a `setTimeout(0)` whenever `ctx.params.section ===
    'recovery'`.
  * A language switch re-renders the page with the same params and `keepScroll: true` (`settings/main.js`
    `onLangChange` → `show(current.id, {params: current.params, keepScroll: true})`). The timer then overrides the
    kept scroll position.
  * It is rarely reachable: the user is usually already at that group after an import or reset. Drop `section` from
    `current.params` after the first scroll, or skip the scroll when `keepScroll` is set.
* **n2, mock errors still lose `data` (C2).** `mock/index.js` `mkErr` (`:42-49`) copies `key` and `params` but not
  `data`, so `bridge.mockCall` passes `undefined`. My scratch check shows `settings.set` with "fr" has no `{path}`
  through the mock router, although `mock_settings.test.mjs` asserts it at module level. Nothing in the UI reads it
  today; copy `data` too (one line), or say so in MOCK-CHANGES.md.
* **n3, wording of the restart-row description.** Windows 11 24H2 says "Show a notification when your *device*
  requires a restart to finish updating" ("je apparaat"). The step chose "your PC" and "je pc" to match the page. That
  is acceptable; it is listed so Robin sees the deviation.
* **n4, D-G31's import rule is ambiguous.** The decision says "the current one stays, as the mock and H do", but the
  old mock and H fall back to the default "nl". The step implemented "the current one stays", and the host's
  `import()` still takes "fr" (`settings.rs:242-268`: sanitize checks types only). The coordinator should confirm the
  rule before R3-NATIVE aligns the host.
* **n5, a known context row silences a real error.** The `known` row for `host/src/services/disks/data.json:unavailable`
  ("Disk management doesn't work") hides a real glossary deviation in check.py. It is documented and reported by
  `glossary.py`, and it stays for its owner and Robin. It should go when the disks oracle text is fixed.
* **n6, cross.md §7 item 9 is not in the note.** The extension's Ctrl+Alt+Tab group "Win11 taskbar" against "Win11
  Taskbar" is marked "round 3 English review". It is in `extension/`, not R3-UI's files, but r3-ui.md §8 does not
  hand it on. Coordinator: give it an owner.
* **n7, stale comments, already noted by the step.** The header of `mock/update.js` (`:38-39`, outside the GW4
  section) still says thrown errors carry only code and message. `notifcenter.js:49` still names "de Win11
  Shell-sessie": a D-G2-class leftover that never shows in GNOME. Both are listed in r3-ui.md §8.
* **n8, GW5 merge risk, as the step says.** The Retry context row covers `update-release.js`. GW5's file has no
  retry text today, in both W and `.wt/gw5`. Any new GW5 text that misses a glossary phrase now fails check.py. The
  coordinator runs check.py right after the GW5 merge.
