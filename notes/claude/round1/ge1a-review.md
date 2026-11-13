# GE1a review: Dutch unchanged, English quality, robustness (2026-11-05)

Adversarial review of step GE1a (`notes/claude/round1/ge1a.md`), with the GE1h harness. G = this repo, H = the frozen
Hyprland tree. I changed no product file. I wrote only:

* `tests/gnome/scenarios/ge1a-switch.py` (new, the scenario the task asked for) and its run output
  `tests/gnome/scenarios/ge1a-switch.out/` (results.json, PNGs, logs);
* scratch dumps in `.rt/ge1a-review/` (gitignored);
* this note.

Every Chrome and GNOME run went through `tools/heavy.sh` (GNOME with `--min-mb 2560`). Nothing ran outside the
sandboxes; no win11 binary, gnome-shell, gsettings, dconf or installer touched the live session.

## Verdict

* **No blocker.** Dutch is byte-identical apart from the listed keys; nothing in GE1a's scripts or tests can reach
  Robin's session.
* **F1 is fixed in the real product.** The hidden preloaded Start opens in the new language from its first DOM frame
  and its first visible frame (nl→en by the dropdown, en→nl by rename); Start open *during* a switch is also right.
* **One major gap:** after a live switch, the window chrome of every app that does not reload (Instellingen,
  Taakbeheer, Schijven; same code in Standaard-apps and the installer) and three taskbar/desktop aria-labels stay
  Dutch (M1). A fresh English window has none of it. This is the same class as F1, it is pre-existing at 3b898e6
  (so not Dutch drift), and it makes the scenario's "every surface shows English" fail.
* The rest are minor: one unlisted Dutch rendering change (C5), verification hygiene (worktree source), and English
  polish.

## 1. Dutch unchanged (my own runs)

| Check | Command (from G) | Result |
|---|---|---|
| nl UI dump | `tools/heavy.sh --tag ge1a-review-dump -- env HEAVY_LOCK=$G/.rt/ge1a-review/inner.lock tests/i18n/run.sh --out .rt/ge1a-review/nl.txt` | 20,460 lines, sha256 `c39159e6…` = GE1a's re-recorded `baseline-nl.txt` (deterministic) |
| …against 3b898e6 | blocks of `.rt/ge1a/baseline-nl.3b898e6.txt` (sha `5db4b8f9…` = `git show b431c627:tests/i18n/baseline-nl.txt`) vs my dump, line diff per block | 139 → 159 blocks: 102 identical, **37 changed, 0 removed, 20 new**. The changed lines are exactly 37 × the breadcrumb aria-label (`Breadcrumb` → `Kruimelpad`) and 1 × `nightNA` on `settings.display`. **0 other changed lines.** |
| New nl blocks | English-word scan (`/usr/share/dict` minus `dutch-words.txt` minus the 3b898e6 nl vocabulary) over the 20 new blocks | 0 hits |
| Dictionary level | `python3 -B tools/i18n/check_ui.py --dict-diff tests/i18n/dict-nl-3b898e6.tsv` | 2,925 → 3,032 rows; 7 changed, 17 removed, 124 added; **0 not in BASELINE-CHANGES.ui, 0 stale** |
| Added rows really unchanged | each "moved literal" row grepped in `git archive 3b898e6` (`.rt/ge1a/base-3b898e6`) | bridge.js 13 texts, tabtransfer/explorer tab texts, `firstNoteNative`, `exportName`, `q_todayQuery`, `homeFolder`, `qq_*`: all byte-identical to 3b898e6 literals. Composed texts re-derived: `freedPartly` = old `` `${freed} – ${partly.toLowerCase()}` ``; `scaleLayoutN` = old `scaleLayout · displayN`; disks `b_*` = old `'{op} {p}'.trim()` with `op_*`; `hintHidden` via `withLink()` = old text + space + link. All equal. |
| FALLBACK_SETTINGS keywords | read `search.js:352-362` | nl keyword strings lost their English words, but `buildSettingsIndex` searches nl + en together, so Dutch search behaviour is kept (it even gains a few new en keywords; fallback path only) |
| Collators | `startmenu.js`, `desktop.js`, `search.js`, `quicksettings.js` | `new Intl.Collator('nl'/undefined)` and `localeCompare` → cached `collator()` with nl-NL. desktop.js used the process default (en-US here); with `sensitivity: 'base', numeric: true` nl-NL and en-US order the same. No visible change found. |
| Decisions | D-G1a kept (13 `.one` forms + about.js + `countOf`), D-G1b held back (`Nederlands`, `Afbeelding-afbeelding`, `item(s)`), D-G2 texts | all as listed; each `.one` row says "on no dump surface", which the dump confirms |
| Rust drift (not GE1a's, for completeness) | `python3 -B tools/i18n/drift_rust.py` | 13,450 frozen literals, 0 errors |
| Other gates | `check.py`; `ratchet.py`; `node tests/i18n/core_i18n.test.mjs`; `TZ=Europe/Amsterdam python3 -B tests/i18n/intl_jsc.py`; `python3 -B -m unittest tests/i18n/test_ge0a.py`; `tools/ui-manifest.py --check`; `scripts/verify-ui.py` | 0 errors / 54 glossary warnings; ok (89 keep); `ok 24214`; 66 values, 0 mismatches; 21 tests OK (2 heavy skipped; the heavy compares are the dumps above); up to date, 177 files; OK (61 changed, 16 added) |
| Real product, nl | scenario phase 1 and 4 (below) | fresh nl and nl after the round trip: every surface `lang=nl`, 0 stale English words, Start headings Dutch |

Unlisted Dutch change found: **m1** (empty notification app name). Nothing else.

## 2. English quality

I diffed every en dictionary value against 3b898e6 (`jsdicts.mjs` on both trees: 80 en values new or changed) and
read them, plus the real-product English text of every surface in the scenario.

* **Good:** en-GB throughout the new texts ("Personalisation", "colour", "holiday", "Recycle Bin"); plurals right
  (`.one` forms read naturally: "End 1 process?", "At most 1 character."); placeholders match nl (check.py: 0 errors);
  no Dutch in any UI string (the en dump's 144 Dutch-word lines that are not copies of nl lines are all mock host
  data: mock apps' Dutch descriptions, mock disk names, mock search and installer messages); D-G2 texts are clear and
  correct for GNOME ("Night light can’t be set up here. Use Ubuntu’s Settings app instead."); D-G5 kept in the UI
  (`firstNoteNative` says "Alles").
* **Glossary:** "Task View" → "Task view" (taskbar tooltip, search action) follows `glossary.tsv` row 8 and the
  Windows 11 Settings toggle; Windows' own taskbar tooltip writes "Task View". Coordinator's call (GE1a decision 3);
  I would keep the glossary.
* **"Shut down or sign out"** (Power page group of lock/sign out/sleep/restart/shut down): matches Windows' Win+X
  submenu; nl "Aan/uit" is Windows' Dutch for the *Power* button, so the two are not equivalents, but both read well.
  Fine.
* Polish items are listed as m4/m5 below.

## 3. Robustness and contracts

* **C1:** no `navigator.language`; `theme.js` caches the last Win11 language in localStorage only for the
  `settings.get`-failed path (still Win11's own setting; unknown → nl). OK.
* **C2:** `bridge.js` and `taskmgr/hostcall.js` copy `key`/`params`; Bluetooth now matches only `code`/`key` (G's host
  sends `code: not_ready`, `bluetooth.rs:340,1261`, so nothing is lost in Dutch). The Wi-Fi `/password|key|secret/i`
  fallback stays (m6).
* **C4:** UI never sends `setTitle` for a native window. OK.
* **C5:** UI renders an empty `appName` as "Melding"/"Notification" (m1 for the unlisted Dutch side).
* **C8:** see decision 8 below; GE1a's `HEAVY_LOCK` inner-lock recipe is correct for `run.sh`.
* **Live-session safety:** `tools/ui-manifest.py` writes only `vendor/ui-sha256.json` and `vendor/ui-patches.json`;
  `check_ui.py` writes only the snapshot it is asked for; `core_i18n.test.mjs` copies into `.rt/i18n-test` and
  removes it; `intl_jsc.py` starts only a JavaScriptCore context (no WebKit process, display or D-Bus); GE1a's
  scratch `.rt/ge1a/run-page.sh` runs Chrome inside `private.sh`; `ge1a-start-switch.py` and my `ge1a-switch.py` run
  only inside the harness sandbox. Test hooks (`?open=`, `demoLauncherQuestion`, Taakbeheer `restart-shell`) are
  dropped by `bridge.js:77-78` without `?debug=1`, and `restart-shell` runs only in mock mode. **Nothing can touch
  Robin's session.**
* **Live switch races:** Verkenner reloads 400 ms after a switch and keeps both tabs (checked both ways); Alles
  reloads at once and keeps its query; Schijven and Taakbeheer drop in-flight answers in the old language
  (`loadGen`/`inflightLang`). A rename in progress in Verkenner is still lost on the reload (pre-existing, not
  tested).
* **Performance:** Start's relabel is a few `textContent` writes; Intl formatters are cached per locale.
  Memory with 8 WebViews open (panel, desktop, Start + 5 apps): peak PSS 1,562 / 1,542 MiB (runs 1/2), lowest
  MemAvailable 3,961 / 4,144 MiB (GE0c: 1,088 MiB with 5). Fine here, but above plan-critique F32's "cap 3 WebViews" suggestion.

## 4. The scenario `tests/gnome/scenarios/ge1a-switch.py`

```bash
tools/heavy.sh --min-mb 2560 --tag ge1a-review -- python3 -B tests/gnome/harness.py launch run --lang nl \
    --build-dir tests/build/3b898e6-debug --scenario tests/gnome/scenarios/ge1a-switch.py
```

1. nl fresh: panel, desktop, Start (opened, then hidden), Verkenner with two tabs (real "Nieuw tabblad" click),
   Instellingen (home, display, clipboard, language), Taakbeheer, Alles, Schijven; each surface's innerText,
   placeholders, aria-labels, data-tooltips and title recorded.
2. The real Taal en regio dropdown (real Clutter pointer clicks, no DOM fallback) to English with Start hidden; Start
   opened 1 s later; DOM recorder per animation frame plus 16 back-to-back screenshots.
3. Every surface polled until it has no word of `tests/i18n/dutch-words.txt` (two clean reads in a row, up to 12 s);
   `ALLOW` is empty, no name needed one.
4. Atomic rename back to nl with Start hidden (first frames again); every surface must be Dutch again: `lang=nl` and
   no English word that appeared in the en state and not in fresh nl.
5. Start open *during* a rename to en.
6. Reference: each app closed and opened fresh in en; "only after a live switch" = live-en hits minus fresh-en hits.

**Results** (run 2, 15:47:35-15:52:06, the final one; run 1 at 15:35 gave the same 31/45 before I added the
visible/aria split): exit 0, 271 s wall (72 s install under strace, 0 writes outside `run/`, 12/12 binaries and 9/9
extension files identical), no leftover process, peak PSS 1,542 MiB, lowest MemAvailable 4,144 MiB, 71 PNGs.
**45 checks: 31 pass, 14 fail, all 14 are M1.**

| What | Result |
|---|---|
| Dropdown nl→en | real pointer clicks, settings.json replaced by rename; `<html lang>` reached en (upper bounds, polled in turn): panel 0.68 s, desktop 0.78, Verkenner 0.84, Settings 0.89, Taakbeheer 0.99, Alles 1.10, Schijven 1.20, Settings page title 1.41 |
| Hidden preloaded Start, opened 1 s after the dropdown switch | 27 DOM frames, **the first one already English**, none dirty; PNG frames: Settings shows through for 6 frames (0-0.16 s) while Start maps, the first Start pixels (frame 07, 0.23 s) are English ("Pinned", "All apps", "Recommended", "Search for apps, settings and documents") |
| en, every surface (two clean reads in a row, up to 12 s) | **clean:** Start (0.11 s), Verkenner (0.11 s, both tabs kept: `robin`, `Home`), Alles (0.07 s). **Not clean after 12 s:** panel, desktop, Instellingen (4 pages), Taakbeheer, Schijven: M1 |
| Fresh en reference (each app reopened) | 0 Dutch words on every app: everything M1 lists exists only after a live switch |
| Rename en→nl, Start hidden | lang nl: panel 0.07 s … Schijven 0.61 s, page title 0.77 s; Start: 32 DOM frames, the first already Dutch |
| nl after the round trip | every surface Dutch within 0.06-0.13 s, 0 stale English words, Verkenner kept both tabs (`robin`, `Start`) |
| Start open during a rename to en | clean after 0.12 s, 0 Dutch words |

The 3b898e6 host shows no Dutch in English mode on these surfaces (the Rust texts reached are already bilingual), so
every remaining hit is a UI string.

## 5. Findings

### Blocker

None.

### Major

**M1. A live switch leaves Dutch window chrome and taskbar/desktop labels behind (F1 class, outside Start).**
* **Evidence:** `ge1a-switch.out/results.json`, checks "en after the dropdown switch: …" (14 failed checks; fresh en has
  none of these words, so `onlyLive` = all of them):

  | Surface | Visible (innerText, placeholder, title, hover tooltip) | aria-label only |
  |---|---|---|
  | Instellingen (all 4 pages) | tooltips "Terug", "Minimaliseren", "Maximaliseren", "Sluiten" | "Navigatie openen", "Wissen", "Zoeken", "Instellingen" |
  | Taakbeheer | tooltips "Minimaliseren", "Maximaliseren", "Sluiten" | "Wissen", "Zoeken", "Taakbeheer" |
  | Schijven | tooltips "Terug", "Minimaliseren", "Maximaliseren", "Sluiten" | – |
  | panel | – (its tooltips do switch: "Task view", "Show desktop") | "Taakbalk", "Taakweergave", "Bureaublad weergeven" |
  | desktop | – | "Bureaublad" |
* **Where:**
  * `vendor/ui/js/apps/appframe.js:80,88-90`: Back (`IconButton` label/tooltip "Terug"), the caption buttons'
    `aria-label` and `data-tooltip` ("Minimaliseren", "Maximaliseren", "Sluiten") are set once; `appframe.js` has no
    `onLangChange` at all (`:147` relabels the max button only on a maximize/restore). The tooltip shows on hover.
  * `vendor/ui/js/core/components.js:617` (TextBox clear "Wissen"), `:664` (SearchBox button "Zoeken"), `:1140`
    (NavigationView toggle "Navigatie openen"; `:1301` relabels it only on open/close); the navigation tree's
    aria-label is the app title passed once by `appframe.js:118` ("Instellingen"/"Taakbeheer").
  * `vendor/ui/js/shell/taskbar.js:1051-1062`: updates the tooltips but not the `aria-label` of the Task view button
    (`:78`), the Show desktop button (`:99`) and the bar itself (`:106`, "Taakbalk").
  * `vendor/ui/js/desktop/desktop.js:57`: the desktop listbox `aria-label` "Bureaublad".
  * Verkenner and Alles are clean because they reload.
* **Not a regression:** the same code at 3b898e6; GE0c/GE1h never read these surfaces' aria-labels after a switch.
* **Fix (UI only, no CSS):** an `onLangChange` in `createAppFrame` that relabels back/min/max/close (aria-label and
  `dataset.tooltip`; max through the same maximized-state function as `:147`); components that set a label once
  subscribe with `onLangChange` and unsubscribe when their element leaves the DOM (or a small registry of
  `{el, attr, key}` in components.js); `taskbar.js` also sets the three `aria-label`s; `desktop.js` relabels the
  listbox. Then `ge1a-switch.py` must pass with 0 failed checks (turn its en checks into `h.require` once green).

### Minor

**m1. Unlisted Dutch rendering change (C5).** `toasts.js` `present()` and `notifcenter.js:235,284,290` now render an
empty `appName` as `t('notification')` ("Melding") where 3b898e6 rendered nothing (toast header, group header, card
title when there is no summary, aria-label). Today only third-party notifications with an empty app name hit it (the
3b898e6 host stores "Melding" itself), so Dutch users see "Melding" in a place that was empty. It is C5's intent, but
`BASELINE-CHANGES.ui`'s own header requires a line for "a change of rendering code that no single key carries", and
its header wrongly counts `toasts/notifcenter notification` among the "moved literals". Fix: add
`GE1a<TAB>shell/toasts.js,shell/notifcenter.js<TAB>C5: an empty appName renders "Melding" (was empty) …` and drop
those two names from the header's moved-literal list.

**m2. `BASELINE-CHANGES.ui` calls D-G1a/D-G1b/D-G2 "Robin's decisions".** `plan.md` lists them as coordinator
defaults (none is marked Robin). Reword to "coordinator defaults (plan.md), offered to Robin in the final report",
so the final report does not present them as already accepted.

**m3. GNOME runs from the working tree mix other steps' in-flight files.** `--source worktree` installs `extension/`
and `scripts/install.py` as GE1c has them at that second. `extension/extension.js` has mtime 15:22:35.785, the very
start of GE1a's passing run (startedAt 15:22:35); GE1a's failed first run (15:15:36-15:18:42, "no WebView boot
report within 90 s") installed an earlier GE1c state. So "an unchanged re-run passed" did not re-run the same tree,
and the failure should not be filed as a flake of GE1a's code; its most likely cause is a half-done GE1c edit (not
provable now: `run/` and the `.out/` were overwritten). My two runs (15:35, 15:47) installed GE1c's 15:22 state,
identical before and after each run (`install.extensionIdentical` 9/9). Fix: re-run `ge1a-start-switch.py` and
`ge1a-switch.py` after the round-1 commit with `--source <commit>`; for parallel rounds add a harness option that
overlays only `vendor/ui` from the worktree on a `--source <rev>` export (GE1h's file).

**m4. English polish (round 3; most strings were already in 3b898e6 as inline ternaries):**
* `core/bridge.js` `badResponse` "Unexpected answer from Win11 Shell." → "Unexpected response from Win11 Shell.";
  `launchRuns` "It runs:" → "This will run:".
* `alles/i18n.js` `tp_word` "…, also “Holiday photos”." → "…, including “Holiday photos”.".
* `explorer/i18n.js` `tabMoveFailed` "Tab could not be moved" → Windows' style "Couldn’t move the tab".
* en-GB spelling left over (pre-existing): `settings/pages/colors.js:62` `captionButtonsDesc` "Minimize, maximize" →
  "Minimise, maximise" (appframe already says "Minimise"); `taskmgr/pages/settings.js:31` `onTopDesc` "maximized" →
  "maximised".
* Schijven in GNOME shows the UI heading "Disk Management doesn’t work" (H's capital M, glossary) over the host body
  "Disk management doesn't work: UDisks isn't available on this computer." (lower-case m, straight apostrophe, and
  the clause repeats the heading). Host text: R2-B should align it with the glossary or drop the repeated clause.

**m5. GNOME-wrong texts still shown (D-G2 class, GE1a decision 7).** `taskmgr/pages/settings.js:15/31` `onTopDesc`
("Werkt alleen in de Win11 Shell-sessie …") is visible in Taakbeheer › Instellingen in GNOME, and its toggle does
nothing (nothing reads `taskmgr.alwaysOnTop`). `notifcenter.js` `unsupported` is reachable only when notifications
are unsupported (not the GNOME case). The Clipboard page keeps the "Clipboard history" toggle and "Press ⊞ WinV …"
under its new "isn’t available here" note. Recommend: rewrite or hide `onTopDesc`/the toggle in the fix step (D-G2
already covers it); the rest in round 3.

**m6. Wi-Fi password heuristics.** `quicksettings.js:869-871`: (a) the kept `/password|key|secret/i` test also matches
the UI's own English `unauthorized` text ("…does not have a valid key."), so in English an expired token on a secure
network opens the password field (in nl the new code no longer matches "sleutel", an improvement); (b)
`password != null && code === 'bad_args'` also fires for the host's other `bad_args`, "Ongeldige netwerknaam."
(`network.rs:328`). Fix: apply the text test only to NetworkManager's own error (`code === 'failed' && reason ===
'dbus'`), and the `bad_args` rule only when the SSID is valid (≤ 32 UTF-8 bytes).

**m7. Not covered yet (left, with owners):** a switch within 150 ms of opening Start (host app names come by the
150 ms catalogue poll; my runs waited 1 s); Alles while indexing; Taakbeheer/Schijven with a slow in-flight call;
Verkenner tab transfer and the picker during a switch; the width audit in WebKitGTK (GE1a's one en overflow,
"2560 × 1440 (recommended)", cannot show in the harness: its virtual monitor offers 1440 × 900). Round 3.

## 6. GE1a's decisions, my view

1. **Kruimelpad:** keep it. It is aria-only (no pixel changes), it fixes English in the Dutch UI, and it is the
   Microsoft Dutch term. The 37 blocks change only on that line (checked).
2. **D-G1b offers to Robin:** fine as listed.
3. **Wording:** "Shut down or sign out" yes; "Task view" per glossary; D-G2 texts read well.
4. **Resolution dropdown:** round 3 in WebKitGTK, agreed.
5. **Retire `tests/i18n/BASELINE-CHANGES`:** yes, once `test_ge0a.py` no longer reads it.
6. **`run.sh` and an ancestor heavy.sh:** yes; same recipe as the harness (GE0a's file, so the coordinator assigns it).
7. **GNOME-wrong texts:** see m5.
8. **"Overlapping" heavy jobs:** not a lock defect. GE1b ran `WIN11_HEAVY_LOCK=$G/target/ge1b-heavy-wrapper.lock
   tools/heavy.sh … -- tests/rust-sandbox/run-tests.sh` (`ge1b.md:64,77`), so its heavy.sh held another file and
   `run-tests.sh` waited for the real lock inside: `ge1b-test` started 15:11:37, ran 3:58.9 and ended 15:15:36, 18 s
   after `ge1c-installer` released at 15:15:18. The cargo work was serialised, but GE1b's memory check ran before the
   real lock (F33). The heavy.sh STAMP in the `.time` file name is taken *before* the lock, which is what made the
   timelines look overlapping. Fix with item 6 (one ancestor detection for `run.sh` and `run-tests.sh`).

## 7. Deviations

* `tests/gnome/scenarios/ge1a-switch.py` matches GE1a's allowlist glob `tests/gnome/scenarios/ge1a-*`. The task let
  me create it; the coordinator should hand it to GE1a's fix step.
* Scratch output in `.rt/ge1a-review/` (gitignored): `nl.txt`, `en.txt` (the two dumps) and an inner lock file.
