# GE1b review: Rust i18n core, C4/C5/C6, the round-2 gate (2026-11-05)

Adversarial review of GE1b (`notes/claude/round1/ge1b.md`, `notes/claude/round2-gate.md`). G = this repo. Nothing
committed. No product file edited. The only files I wrote in G: `tests/gnome/scenarios/ge1b-core.py` (new),
its run output `tests/gnome/scenarios/ge1b-core.out/`, this note, and run logs under `tests/logs/` and
`tests/rust-sandbox/logs/` (both ignored or run output); the harness rebuilt its shared `tests/gnome/run/` on each of my three runs. Mutation tests ran on a copy in my scratchpad, never in G.

**Verdict: no blocker.** Dutch is unchanged apart from the 12 listed title keys. The host side of C2, C4, C5 and C6
works in the real product (private GNOME Shell, WebKitGTK, GE1b's binaries). I found 2 majors, both gaps in the
round-2 gate rather than defects in what GE1b converted, and 10 minors.

## 1. What I ran (from G)

| Check | Command | Result |
|---|---|---|
| Drift | `python3 -B tools/i18n/drift_rust.py --summary --warnings` | 13,450 frozen literals in 116 files; 13,390 present, 60 mapped or listed (runtime 44, services/mod 5, window 11); 27 rows; **0 errors, 0 warnings** |
| Freeze reproducible | `drift_rust.py --freeze` on a scratch copy (`.git` symlinked read-only) | 13,450 literals (12,454 Rust, 996 JSON); sha256 `e72ad6ad75966c02…`, **byte-identical** to the committed file |
| GE0b's frozen nl inside it | script: every (file, text) of `tests/i18n/rust-nl-frozen.tsv` in `rust-frozen-all.tsv` | 1,079 of 1,079 present, 0 missing |
| Checker | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` | 24 stems, 27 rows, **0 errors, 0 warnings** |
| Tests + oracles | `WIN11_HEAVY_LOCK=$G/tests/logs/ge1b-review-wrapper.lock tools/heavy.sh --min-mb 2048 --tag ge1b-review-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` (GE1b's recipe, deviation 1) | rc 0. lib **269 passed, 8 ignored**; win11-launch **5 passed, 1 ignored**; 11 other bins 0 tests. Oracles **16/16** (shell 8, disks 7, system 1). `guard-calls.log` 0 bytes. 2.4 s wall (cached build), 254 MB peak RSS |
| Desktop-entry table vs installer | script: `git show 3b898e6:scripts/install.py` titles and Comment against `host/src/desktop-entries.json` nl | 10/10 names and the comment byte-identical |
| Binaries tested | `cmp` `tests/build/r1-debug/*` against `target/debug/*`, `sha256sum -c SHA256SUMS` | 12/12 identical and OK; built 15:27, after the last host edit (15:26) |
| Real product | `tools/heavy.sh --min-mb 2560 --tag ge1b-review -- python3 -B tests/gnome/harness.py launch run --lang nl --build-dir tests/build/r1-debug --scenario tests/gnome/scenarios/ge1b-core.py` (3 runs, section 3) | see section 3 |

**Not run, on purpose:** the UI dumps and the dictionary diff (`tests/i18n/run.sh`). They run the mock UI in Chrome
with no Rust host, and GE1b changed nothing under `vendor/ui`, so they cannot see GE1b's work. The Rust side of
Dutch is covered by the drift check, the render test and the oracles. The real-host Dutch (titles, error replies,
entries, notification label) is covered in the GNOME run.

## 2. Lens 1: Dutch unchanged

* **Every Dutch difference is listed.** `BASELINE-CHANGES.rust` has 12 `key:` lines (the new Dutch native titles,
  C4/D-G6), 3 English title replacements and 37 technical literals (json keys and `"language"`/`"en"`/`"nl"`).
  None of these is a D-G1a/D-G1b/D-G2 case. No other nl text changed: drift found 0 errors, and the render test
  `every_converted_site_renders_its_3b898e6_dutch` passes.
* **In the real product, Dutch was byte for byte the 3b898e6 text:**
  * the error messages of 5 forced errors in 3 processes;
  * all 10 desktop entries after en → nl, which were identical to what the installer wrote (sha256 per file);
  * the "Melding" label.
* **Dutch the user will now see that 3b898e6 never showed** (all listed, all mandated by C4): native titles
  "Instellingen", "Verkenner", "Taakbeheer", "Schijven", "Standaard-apps", "App-installatieprogramma", "Win11
  Taakbalk", "Win11 Bureaublad", and the picker titles "Openen", "Opslaan als", "Map kiezen". GNOME's overview,
  Alt+Tab and "“…” is ready" banners now show these. Robin should see them in the report.
* **JSON shape change.** The 403/405/400/411/413/502 replies and `unknown_method` now carry `key` and `params`.
  `code`, `message` and `data` are unchanged. This is additive, as C2 says.

## 3. The GNOME scenario (`tests/gnome/scenarios/ge1b-core.py`)

The run installs the GE1b binaries with `--source worktree`, so the UI (GE1a), the extension and `install.py`
(GE1c) are the current working tree. Tree hashes at run time: vendor/ui `4f711452…`, extension `21fb6f22…`,
scripts `b704c35e…`, host/src `cb55e08f…`.

**Parts:**
* **A, Dutch:** Settings (Taal en regio), Verkenner, Taakbeheer, Alles and Schijven open. Then 5 forced errors
  each in Settings and in the panel (unknown method, a body that is no JSON, a bad method name, non-object args, a
  GET on `/api/call`), plus `bridge.call` of an unknown method. Then two GNOME notifications, from a source titled
  "Melding" and from one without a name. Then the 10 entries and GNOME's app names.
* **B:** the real dropdown switch to English. Titles, errors (Settings and Verkenner), entries, app names, the
  notification centre, and two toasts posted in English.
* **C:** the dropdown back to Dutch. Everything is checked again, and the entries must be byte-identical to A.
* **D:** two file pickers from Settings, one without a title and one with the caller's title "Een foto kiezen voor
  je bureaublad". Rename to English and back, then close them.
* **E, single writer and repair:** stop win11-panel, rename to English while it is down, and check that nobody
  rewrites the entries. Restart the panel through its own (still Dutch) entry: it repairs the entries at start. Then
  the titles, the notification centre after a fresh list, and Taakbeheer opened fresh in English. Back to Dutch.

**Final run (run 3, 16:10:42-16:13:42): exit 0, 179.9 s wall.**
* **Checks:** 93, of which 90 passed. Of the 3 that failed, 2 carry `known=`: `review:event-path` (m2) and
  `review:picker-own-title` (m3). The third, a picker that does not close on a WM close request, is observation o1.
* **Install:** 87,844 mutating paths, 0 outside `run/`.
* **Memory:** peak PSS 1,352 MiB (whole sandbox, 44 processes), lowest MemAvailable 3,668 MiB.
* **Cleanup:** no leftover processes.
* **Logs:** no `i18n:` miss and no panic in any app log.
* **Dropdown switches:**
  * nl→en by real pointer clicks: `<html lang>` of panel 0.72 s, Taakbeheer 1.08, Alles 1.19, Schijven 1.29,
    Verkenner 1.44, page title 1.59 s.
  * en→nl: the pointer did not open the list, so the harness used its recorded DOM fallback. The fallback is a
    harness flake, not a product failure.
* **C6 timings:** the files were English at the first poll after the switch and Dutch again 0.4 s after `set_language`
  returned on the way back; GNOME's app names followed after 5.1 s and 5.2 s. In part E the files were already repaired when the
  restarted panel was ready, and the names followed after 4.8 s.
* **Title reports:** 38 `title` dev reports. That is one per open window per switch, the two pickers included.

**What the runs showed:**

| Contract | Evidence |
|---|---|
| **C4 titles** | nl: Instellingen, Verkenner, Taakbeheer, Alles, Schijven, Win11 Taakbalk, Win11 Bureaublad. After the dropdown: Settings, File Explorer, Task Manager, Alles, Disks, Win11 Taskbar, Win11 Desktop, already switched at the first poll after every page had changed its `<html lang>` (≤ 1.5 s after the click). Back in Dutch, all of them Dutch again. A fresh Taakbeheer opened in English was "Task Manager" from creation, and the restarted panel was "Win11 Taskbar". The untitled picker followed the switch: Openen → Open → Openen. 26 `title` dev reports in run 1: one per window per switch, no duplicates. |
| **C2 errors** | All 5 errors in 3 processes (Settings, panel, Verkenner) have the 3b898e6 Dutch message, the right `code` and `key`, and `params: {}`. In English: "This function doesn't exist.", "Invalid request.", "Invalid input.", "Use POST.". `unknown_method` keeps `data.method`. `bridge.call` hands `key` to the caller (GE1a). Status codes 200/400/405 as before. |
| **C5** | The extension's own list returns `appName` "Melding" for the "Melding" source and "" for the nameless one. The host's `notifications.list` returns "" for both. The notification centre shows "Melding" in nl and "Notification" in en, because it re-lists on every open (`notifcenter.js` onOpen). **A toast does not:** in English, the toast for the "Melding" source says "Melding" next to a "Notification" toast (`B-en-toasts.png`, minor m2). |
| **C6** | B: all 10 files had the English Name/Comment at the first poll, and only those lines differed. GNOME's `Shell.AppSystem` names followed 4.8-5.1 s later, all 10 ("Win11 File Explorer", "Win11 Task Manager", "Alles", …). Back in Dutch, the files were byte-identical to the installed ones, every file had been replaced by rename (new inode), and no temporary file was left. E: while the panel was down, all 10 files stayed Dutch for 3 s after the rename to English, so the other 5 processes do not write them (single writer, F3). The restarted panel repaired them at start, and GNOME followed after 4.5 s. The rewritten Panel entry still launched the panel. |
| **Safety** | Install wrote 87,844 mutating paths, 0 outside `run/`. 12/12 binaries and 9/9 extension files match. Stub calls in run 3: wpctl 112, systemctl 66, pw-dump 31, apt 2 (all refused). No process left over. No `i18n:` miss line and no panic in any app log. Run 3: peak PSS 1,352 MiB, lowest MemAvailable 3,668 MiB; run 1: 1,386 MiB and 4,349 MiB. |

**Runs 1 and 2** had scenario bugs of mine, both fixed:
* I assumed that pickers have their own WM class. GNOME reports them under the caller's class,
  `io.github.win11shell.Settings`.
* Closing both pickers at once left the untitled one open (observation o1).

Run 1 and run 2 results are kept in my scratchpad only. The final output is in `tests/gnome/scenarios/ge1b-core.out/`.

## 4. Findings

### Blockers

None.

**Live-session safety:**
* None of GE1b's tools touches the session:
  * `drift_rust.py` reads git objects only;
  * `check_rust.py` reads files and runs node on `jsdicts.mjs`;
  * `extract_rs.py` writes only `host/src` and `host/i18n` under `ROOT`, and only with `--apply` (`relative_to(ROOT)`).
* GE1b's new tests use temp dirs, and no test calls `applications_dir()`, `schedule()` or `run()`.
* `guard-calls.log` stayed empty.
* The C6 writer exists only in `win11-panel` and only writes `$XDG_DATA_HOME/applications`. In the harness the
  real HOME is read-only.
* My scenario runs only inside the harness sandbox.

### Major

**M1. The round-2 gate has no Rust ratchet, so new Dutch-only literals pass every check.**
* **Gap.** `drift_rust.py` only proves that frozen literals are still present, mapped or listed. `check_rust.py`
  only checks catalogue rows and their call sites. `plan.md` round 2 names "ratchet clean" as a done-when;
  `round2-gate.md` §6 drops it.
* **Evidence.** On a scratch copy I added `fn extra() -> String { "Deze melding is nieuw en Nederlands.".into() }`
  to `services/notifications.rs`. Result: drift 0 errors, check_rust 0 errors.
* **Why it matters.** Round 2 hand-converts pieces, splits sentences and adds messages. Any new inline Dutch text
  stays Dutch in English and is invisible to the gate.
* **Fix (GE1b's tools).** Add `drift_rust.py --new`: list every non-test literal of the working tree that is not
  in the frozen multiset of its file.
  * Classify each with GE0b's `scan_rust.py` word rules.
  * Fail on user-visible Dutch that is not under `#@`, not allow-listed and not a key.
  * Add it to round2-gate §6.

**M2. Nothing checks for Dutch in the English column of a Rust row.**
* **Evidence.** On the scratch copy, check_rust gave 0 errors and 0 warnings for both:
  * `core.tooLarge` with en = nl ("Te veel gegevens.");
  * en "Te veel data, probeer opnieuw.".
* **Why it matters.** Round 2 brings about 463 rows, of which about 94 have H's unreviewed round-2 English. Rust
  texts are not in the mock UI dumps, so `scan_en.py` cannot see them before the round-3 screenshots.
* **Fix.** Add a check_rust rule `en-dutch` (error):
  * en tokens that are in `tests/i18n/dutch-words.txt`;
  * en == nl when nl has a Dutch word.
  * Allow-list names in `allow-host.txt` (`label:Alles`, "Win11", product names).

### Minor

**m1. `LEGACY_APP_NAME` ("Melding") is a matcher, but the converter offers it for conversion.**
* `extract_rs.py list host/src/services/notifications.rs` shows ordinal 2 at L23 as `user-visible-dutch`.
* If R2-A converts it, the C5 legacy mapping breaks.
* **Fix:** add `host/src/services/notifications.rs	item:LEGACY_APP_NAME	matcher for the legacy stored appName (C5)` to `allow-host.txt`.

**m2. C5 maps "Melding" on load, but not on the `notifications.new` event path.**
* The extension's events reach the page through `gnome.rs` `EventRelay` unmapped. Shown in all 3 runs
  (`B-en-toasts.png`): in English the toast says "Melding", while the centre says "Notification" for the same item.
* No real legacy store exists: the host's fallback store is in memory and maps in `add()`, and the 3b898e6
  extension used "Notification". So in practice only an app that really is called "Melding" hits this, and it then
  shows two different names.
* **Fix:** pick one rule.
  * (a) Map events as well: one `clear_legacy_app_name` in the relay for `notifications.new` (`gnome.rs`, R2-D).
  * (b) My preference: drop the list-path mapping and keep only the `add()` one. This needs a C5 wording change
    (coordinator).

**m3. A picker opened with the caller's own title keeps it across a switch.**
* All 4 in-app pickers pass `title: t(...)`: Settings › Achtergrond, Taakbeheer storage, the installer and Alles
  options.
* In English mode the native title and the picker page's own title stay Dutch, for example "Een foto kiezen voor
  je bureaublad" (`D.en` record; run-1 `E-centre.png` shows it in the window).
* It only lasts while the picker is open during a switch.
* **Fix:** let a caller's own title win only for portal callers (`options.portal == true`). For in-app callers,
  use the mode title or have the UI send a key. The page side is GE1a's.

**m4. The label check in check_rust.py fails open.**
* Without node or `vendor/ui/js`, the UI-label rule is skipped with only a warning (shown on the scratch copy:
  "UI label check skipped: jsdicts.mjs failed"). A round-2 agent would get "0 errors".
* **Fix:** make it an error unless `--no-labels` is given.

**m5. `watch_language` applies the language carried by the event, not the current one.**
* The datagram loader thread and the 150 ms poll thread dispatch `settings.changed` independently. If two quick
  switches are dispatched out of order, `last` and the titles end in the old language and no later event corrects
  them.
* `desktop_entries::schedule` has the same "newest wins" problem: it uses the lang it was given.
* The window is microseconds wide, so this is unlikely, but it is cheap to close.
* **Fix:** in the idle callback and in the delayed entry worker, apply `i18n::current_language()` at that moment.

**m6. `language_in` never trusts a file with an mtime in the future.**
* A file restored from a backup or written under clock skew makes `duration_since` fail, so `trusted` stays false
  and every call parses the file again: the old cost, silently.
* **Fix:** trust it when the stamp is unchanged on a second stat.

**m7. The `setTitle` IPC is still honoured for host-titled kinds (GE1b decision 3).**
* A stray `setTitle` would override C4 until the next switch.
* **Fix:** recommend ignoring it for windows in `TITLED`, and logging it once.

**m8. Names differ between GNOME's grid and the window titles. This is pre-existing and affects Dutch too.**

  | | Desktop entry (grid) | Window title |
  |---|---|---|
  | Schijven | "Win11 Schijfbeheer" / "Win11 Disk Management" | "Schijven" / "Disks" |
  | Installer | "Win11 App-installatie" | "App-installatieprogramma" |
  | Standaard-apps | "Win11 Standaardapps" | "Standaard-apps" |

  * Changing the entries is a Dutch change, so this is Robin's decision (round 3).
  * The English desktop-entry Comment is developer jargon ("Existing Win11Shell interface in a standalone Rust
    host for GNOME"), as the Dutch one is.

**m9. The performance check is narrower than F28 asked.**
* GE1b benchmarked `ctx.lang()`: 2.3 µs against 30.5 µs.
* F28 asked for the sysmon process list to be measured before and after; that was not done.
* **Fix:** add that measurement in R2-B.

**m10. The converter defaults to `crate::i18n::current_language()` when a site has no `@LANG`.**
* In production this equals `ctx.lang()`. In a service test with its own config dir it silently reads the private
  HOME instead, and English assertions then fail far from the cause.
* **Fix:** warn when the enclosing fn has `ctx`, `self.ctx` or `lang` in scope and no `@LANG` was given.

### Observations (not GE1b's files)

**o1. A file picker did not close on a window-manager close request** (`picker.rs`/`window.rs`, R2-D).
* In run 2, `meta_window.delete()` was sent to both Settings pickers in the same call. The caller-titled one
  closed; the untitled one stayed open, and its `shell.pickFile` promise never settled within 20 s.
* In run 3 they were closed one at a time: two close requests to the untitled one (opened first) left it open for
  6 s each, and its promise was still pending 5 s later (`D.closed`, `D.results`).
* This probably matters outside the test too: the picker's own close button is the window manager's.
* Cause not traced.

**o2. Pickers carry the caller's WM class.** Both Settings pickers had `io.github.win11shell.Settings`, not the
`io.github.win11shell.FilePicker` that `picker.rs:157-179` sets. GNOME groups them under Settings. This may be
intended; recorded for whoever owns the harness WM map.

**o3. C5 changes how the centre groups notifications (UI, GE1a).**
* `appKey` = appName or appIcon. Nameless notifications used to share one "Melding" group in the host fallback
  path, or one "Notification" group in the 3b898e6 extension path. They now group per icon.
* All these groups carry the same label.
* **Fix if unwanted:** a fixed key for `""`.

**o4. Deploy and rollback (round 7).**
* On its first start, the new live panel rewrites Robin's 10 `~/.local/share/applications/io.github.win11shell.*.desktop`.
* A rollback to the 3b898e6 binaries does not rewrite them back.
* **Fix:** add these files to the backup and rollback recipe.

## 5. Contracts

| | State |
|---|---|
| C1 | Met: only `settings.language` decides; exactly "en" is English; no `X-Shell-Lang`, no LANG; the one config dir rule shared by i18n.rs, runtime.rs and settings_runtime.rs. Deviation 2 (no `settings.broken-*` rename on the language path) is acceptable: `ctx.settings()` still renames. |
| C2 | Met. Additive `key`/`params` on the HTTP envelopes is consistent with the contract text. |
| C3 | Met: format, escapes, `{name}`, plural rule, nl from G, en from H only on an exact match (7 rows), fallback logged once. |
| C4 | Met for every kind tested live; picker caveat m3; `setTitle` caveat m7. |
| C5 | Host side met for `add()` and the list; event path m2. |
| C6 | Met, including the single writer, the debounce, the start-up repair, atomic writes, the CRLF and foreign-file cases, and byte identity on the way back. |
| C8 | GE1b's deviation 1 still stands (memory check before the real lock wait). I used the same recipe for my test run. The GNOME runs used plain heavy.sh, which the harness recognises. |
| C9 | allow-host.txt covers the listed kinds; missing `LEGACY_APP_NAME` (m1). |

## 6. Decisions for the coordinator (my recommendation on GE1b's seven, plus mine)

1. **Lock:**
   * heavy.sh exports `WIN11_HEAVY_HELD=<lock path>`.
   * run-tests.sh skips its flock (and its pre-lock memory check) when that path equals its own `LOCK` and the
     lock is really held (`flock -n` fails).
   * Owner: whoever unfreezes `tests/rust-sandbox/`.
2. **Titles:**
   * Accept the 12 Dutch titles.
   * Put the grid-vs-title name differences (m8) on Robin's list.
3. **`setTitle`:** ignore it for host-titled windows (m7).
4. **Dry-run:** no rewrite. Agree.
5. **Drift strictness:** keep every literal. Add the ratchet (M1) and the en-dutch rule (M2) before round 2 starts.
   GE1b can do both in its fix step; both are in its own tools.
6. **Disk:** about 1 GB per worktree is fine at 22 GB free; remove worktrees after the merge.
7. **Stems:** agree, plus the `allow-host.txt` line of m1.
8. **New: C5 rule** (m2), mapping on events or only in `add()`.
9. **New: picker own titles only for portal callers** (m3).
10. **New:** add the desktop entries to the deploy backup (o4).
11. **New:** commit `scenarios/ge1b-core.out/`, at least `results.json`.
