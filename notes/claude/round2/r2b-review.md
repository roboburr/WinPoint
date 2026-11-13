# R2-B adversarial review (sysmon, storage, disks, index), round 2, 2026-11-05

Reviewer: a separate agent. W = `.wt/r2-b` (branch `r2/b`, from d666df54). I edited no product file. The only files I
wrote are this note and scratch files under `W/.rt/review-r2b/`: `nlcheck.py`, `sitecheck.py`, `all-literals.tsv`,
the logs, and a `git archive` copy of d666df54 used for a rustfmt comparison. Every heavy command went through
`W/tools/heavy.sh` (tags `r2b-rv-test`, `r2b-rv-bench`).

**Verdict: fix-needed.** There is 1 blocker, 2 majors, 4 minors and 5 notes.
* The text conversion itself is clean. All 169 `#@` records match the 3b898e6 bytes, and there is no Dutch text
  change.
* The blocker is that extra 1's re-render of category names changes the `storage.categories` reply shape, which
  changes what a Dutch user sees on the first scan.
* One Dutch index error was missed.
* The two GNOME done-when runs are still open.

## Findings (most severe first)

### B1 (blocker): `storage.categories` now returns `data: {"categories": null}` instead of `data: null`

Task Manager › Opslag shows an empty chart instead of "Berekenen…".

* **Where:** `host/src/services/storage/categories.rs:68-81`, new in this step (extra 1, "category names re-rendered
  when read"):
  ```rust
  let mut out = json!({"ready":…,"data":state.data,"running":…});
  for category in out["data"]["categories"].as_array_mut().into_iter().flatten() { … }
  ```
* **Why it breaks:** `out["data"]["categories"].as_array_mut()` goes through serde_json's `IndexMut`. When
  `state.data` is `None`, `out["data"]` is `Null`. For a string key, `index_or_insert` turns `Null` into an object and
  inserts the key (serde_json 1.0.154, `src/value/index.rs:96-103`). The reply therefore changes:
  * before: `{"ready":false,"data":null,"running":true}`
  * now: `{"ready":false,"data":{"categories":null},"running":true}`
* **When:** `state.data` is `None` on the first scan ever, or when `storage-categories.json` is missing or corrupt
  (`Categories::new` drops a non-object).
* **What the user sees** (`vendor/ui/js/apps/taskmgr/storage/categories.js:63`, `if (r?.data) { data = r.data;
  render(); }`):
  * the object is truthy, so `render()` clears the "computing" card (ProgressRing plus "Berekenen…" and the file
    counter);
  * it draws an empty donut ("0 B in totaal", home "~", empty legend) until `storage.categoriesReady` arrives, which
    can take up to the 180 s budget.
  * The Settings page is not affected, because it checks `ready`.
* **Why blocker:** the Dutch screen differs from 3b898e6 (and so does the English one), and the wire shape of an
  unchanged API changed. The unit test `categories::tests::cached_category_names_follow_the_language` only covers
  `data: Some`.
* **Fix:** iterate without inserting. Add a test that `get()` with `state.data = None` returns `data == null`, and
  one for a cache object without `categories`.
  ```rust
  if let Some(cats) = out.get_mut("data").and_then(|d| d.get_mut("categories")).and_then(Value::as_array_mut) {
      for category in cats { … }
  }
  ```

### M1 (major): missed Dutch index error "Inconsistente indexkolom" (`host/src/services/index/cache.rs:332`)

* **What:** `Mapping::open` returns `io::Error::new(InvalidData, "Inconsistente indexkolom")`. It is the same family
  as the 8 `Data::load` errors this step turned into `Fault` keys (`index.cache.*`).
* **How it reaches the user:**
  * `Data::load` calls `Mapping::open` for every column (cache.rs:504-511).
  * `cache::build` ends with `Data::load(cache)` (cache.rs:1066).
  * A failed read-back becomes the root error of the status: `engine.rs:272` `cache::root_error(&e)`.
  * `root_error` keeps a non-`Fault` error's text as it is.
  * So an English Alles shows Dutch in `roots[].error` in exactly the case the step's own row note describes ("can
    reach roots[].error of the status only when a fresh build cannot be read back").
* **Why no tool caught it:** GE0b classed the literal technical, so `extract_rs.py list` hides it (only `--all` shows
  it), and the ratchet only looks at new literals. It is the only Dutch-looking literal left in the owned files
  outside the query parser's C9 input words: I grepped `list --all` over all 21 files with `dutch-words.txt` and with
  shape heuristics, saved in `.rt/review-r2b/all-literals.tsv`.
* **Fix:** convert it the same way as its siblings. Rerun list/convert with `--force` (the literal is classed
  technical). Use key `index.cache.badColumn` and en "Inconsistent index column", then
  `fault(io::ErrorKind::InvalidData, "index.cache.badColumn")`. This adds one row and no Dutch change.

### M2 (major, blocked outside the step): done-when items F28 and extra 6 are not met; GE1a's scenario will fail after the merge

* **Not run:** `ge1b-perf.py` (r1 against r2b, `sysmon.processes` median) and the disks-race rerun.
* **The block is real:**
  * the socket path from W is 102 bytes (`…/.wt/r2-b/tests/gnome/run/rt/system-bus`), against 93 from G;
  * `ge1b-perf.out/r2b/r1/logs/system-bus.log` says "dbus-daemon[40]: Failed to start message bus: Socket name too
    long";
  * `harness.py:63` sets `RT = RUN / 'rt'`, and `:906` only asserts `< 104`.
  * The proposed patch is sound: `launch` execs bwrap (`harness.py:618`), and `run` calls `prepare()` (`:1230`)
    inside it, where `/tmp` is a fresh tmpfs (`bwrap_argv` `--tmpfs /tmp`). So `RT = Path('/tmp/w11-rt')` is
    private per run and complies with C8. The symlink route does not work, because `HERE` is `resolve()`d.
* **What exists instead:** only an in-process micro-benchmark of `unit_friendly`. I reran it:
  * nl: 0.539 µs at 3b898e6, 1.497 µs now;
  * en: 0.015 µs at 3b898e6, 1.458 µs now;
  * 58 units, debug build, matching the note's numbers.
  * That is plausibly about 0.1 ms per `sysmon.processes`, but it is not the F28 measurement.
* **GE1a's scenario:** `tests/gnome/scenarios/ge1a-disks-race.py:23` pins
  `HOST_TEXT['en'] = "UDisks isn't available on this computer"` (ASCII apostrophe). R2-B's new body "UDisks isn’t
  available on this computer." will make GE1a's English checks fail on any build after the merge.
* **Fix (coordinator):**
  1. Apply the harness patch (`RT = Path('/tmp/w11-rt')`, assert `< 100`).
  2. Run the four commands of r2b.md §8, decision 1.
  3. Either retire `ge1a-disks-race.py` in favour of `r2b-disks-race.py`, or change GE1a's `HOST_TEXT['en']`.
     `r2b-disks-race.py` compiles and reads correctly but has never run.

### m1 (minor): English wording, three rows

* `storage.trashFailed` (storage.tsv:21) en is "That didn’t work (ApiError)."
  * "(ApiError)" is a Codex leftover in the 3b898e6 Dutch, so nl must keep it, but the en row is new text.
  * Write "That didn’t work." and list the nl artefact for Robin (D-G1b style, held back).
  * Shown in Task Manager › Opslag cleanup results when emptying the Recycle Bin fails.
* `sysmon.restart.noEnviron` (sysmon.tsv:75) en is "This process doesn’t let its settings be read, …".
  * This follows the Dutch word for word: "instellingen" here means the environment.
  * Better: "This process’s environment can’t be read, so it can’t be restarted here."
  * Or: "Win11 can’t read this process’s settings, so it can’t be restarted here."
* `sysmon.restart.parent` (sysmon.tsv:90) en drops the quotes that nl has (‘{parent}’).
  * "started by Web Content and belongs to it" reads poorly. Windows quotes names.
  * Use “{parent}” (the row's en is free), or accept as is.
  * The quoted fallback "another program" is the only argument against.

### m2 (minor): test gaps

* No test of `Categories::get` without data (it would have caught B1).
* The health page's names after a switch are untested (the note says so).
* `storage_volumes_follow_a_switch_within_the_cache_window` is skipped when the root file system is not listed. The
  pure `VolumeCache` test still covers the logic.
* The disks en test goes through `map_error` + `unsupported`, not `Disks::call("disks.list")`.

### m3 (minor): `key:disks.unavailableBody` reports a Dutch change that is none

* **What:** the row is `#@ new` plus a `key:` line in `BASELINE-CHANGES.rust-r2b`, although its nl is 3b898e6's
  `_STR/nl/unavailable` byte for byte. Robin's report would then list it as a Dutch change.
* **Why the stated reason does not hold:** the frozen literal exists as `host/src/services/disks/data.json#66`
  (`rust-frozen-all.tsv`, sha 05e022cb923e…). The note says a mapping "fails drift". In fact
  `drift_rust.py:918-920` gives a *warning* ("mapping/baseline line(s) … although the literal is still present"),
  not an error.
* **Why it matters:** `every_converted_site_renders_its_3b898e6_dutch` skips `#@ new` rows. The byte identity now
  rests only on the step's own disks unit test.
* **Fix:** recreate the row with `extract_rs.py add --stem disks --key disks.unavailableBody … --from
  host/src/services/disks/data.json#66`, drop the `key:` line and explain the one drift warning. Or ask for the
  "copy of a frozen literal" mapping (decision 2).

### m4 (minor): wrong statements in the step's note, BASELINE reasons and a test comment

* r2b.md §8 deviation 4, the summary, and the comment at `sysmon.rs:272-273` say: "restartPlan … in English (was
  `protected`, now `not_found`)".
  * This is false for 3b898e6. Its `check_own` returned Dutch text in both languages, so "bestaat niet" matched and
    English already got `not_found`.
  * The `Own` refactor is right, but it is no behaviour change. Reword it as "keeps `not_found` in English, which a
    plain conversion would have broken".
* The BASELINE reason for `index.rs#184` says 3b898e6 wrote "1,5 TB-schijf" / "1,5 TB Drive".
  * At 3b898e6, `index.locations` labels were Dutch-only in both languages ("Systeemschijf", "1,5 TB-schijf").
  * "System drive" and "1.5 TB Drive" are new English, not a changed English separator. Dutch is unchanged, so the
    class is right.
* "fails drift" (decision 2) should read "gives a drift warning" (see m3).

### n1 (note): the `query.rs` allow rule is narrow and needed, but its reason is not quite true

* `fn:new&class:user-visible-english` matches exactly the 5 regex details ("nothing to repeat" …), which is narrow.
* They are user-visible: they appear inside the Dutch message "Deze reguliere expressie klopt niet: nothing to
  repeat.". They are not "technical", and not C9.
* Keeping them is correct, because translating them would change Dutch. Say that in the reason ("user-visible
  English detail of 3b898e6, kept so Dutch stays byte-identical; translation would be a Dutch change, round 3").

### n2 (note): `process.rs:517` keeps `if lang == "en" { en } else { nl }`

`own_names` is used here, while `actions.rs` uses `i18n::pick` for the same table. This is harmless, but use `pick`
for consistency. Decision 4 (reuse `window.title.*`) stays for round 3.

### n3 (note): `sysmon.health` now builds a full process list on its first call

* `health.rs:19` runs `if pm.names_lang() != lang { pm.processes(ctx, false); }`, and `names_lang` is "" at first.
  The first call therefore runs a full `processes()` (about 36 ms debug).
* On that first call `high_cpu(30.)` is almost always empty, because a process must have been hot for 30 s, so the
  work is wasted. In 3b898e6, names for busy processes came from comm when no list had been built.
* Cheaper, and identical to 3b898e6 when nothing is busy: rebuild only when `!hot.is_empty() && names_lang != lang`.

### n4 (note): two English texts for "UDisks missing"

* The `disks.list` body is the new "UDisks isn’t available on this computer.".
* Action errors that go through `map_error` keep the oracle-pinned "Disk management doesn't work: UDisks isn't
  available on this computer." (ASCII ', lower-case m) in toasts titled "failed".
* This is documented by the step. The `_STR` table's D-G23 conversion needs a round-3 decision, because the disks
  oracle pins 214 English cases.

### n5 (note): decisions 3, 5 and 6 are fine

* Decision 3: GSD/GVFS part names as text.json `{nl, en}` data, like `_FRIENDLY`, is fine. All 23 nl values are
  byte-identical to 3b898e6's `_GSD_NL` / `_GVFS_NL`; I compared them by script.
* Decisions 5 and 6 are confirmed: `query.rs:859` `date_word` accepts `"today" | "vandaag"`, so gate §5's sentence
  is wrong for G. `the_examples_of_the_messages_parse` parses every example used in the messages.

## What I verified and found correct

* **Dutch byte identity (check 1):**
  * `nlcheck.py` (mine) reads every `#@` record of the 4 stems and checks it against `rust-frozen-all.tsv` and the
    3b898e6 source line:
    * 169 records (146 plain, 18 fmt, 3 en, 2 enfmt) plus 1 `#@ new`;
    * 0 differences after reversing the placeholder renames;
    * every sha prefix matches.
  * `sitecheck.py` (mine) checks that each key is called in the same function as its 3b898e6 literal: 34 keys moved,
    all to explained hand conversions (`Own::key`, `confirm`, `UNIT_NAMES`, `gave_up_status`, `worker_error`), and
    0 absent.
  * I read every one of the 18 fmt call sites: the arguments equal 3b898e6's `{}` arguments (Display text). No row
    has more than one placeholder, so argument order cannot go wrong.
  * The `UNIT_NAMES` unit→key→nl map equals the 3b898e6 `unit_friendly` table (28 of 28). The test's copy of the
    3b898e6 function equals 3b898e6 apart from the table rename.
  * Hand-checked logic:
    * `fmt_duration`: 1 minuut, n minuten, `{} u {} min`;
    * `size_label` (nl: "1,5 TB", "2 TB");
    * `localize` ("Overige" from `_STR`);
    * the categories re-render (same `c_{id}` key as `compute`);
    * disks `unsupported`/`list` (the `_STR` unavailable text);
    * `worker_error`/`localized` (re-render from the key with the same params).
  * BASELINE-CHANGES.rust-r2b: 13 removed technical literals, each with a true no-Dutch-change class (drift gives
    0 warnings, so each line covers a literal that really disappeared), plus the `key:` line of m3. No D-G1a or D-G2
    line is needed.
* **Language source (check 2):**
  * Services use `ctx.lang()` / `ctx.language()` everywhere.
  * `current_language()` appears only in worker-side code (engine.rs, query.rs, cache.rs `Fault`/`root_error`), and
    the host renders those texts again from key + params.
  * `query::integer`'s host-side error is discarded and rebuilt with `self.ctx.lang()`.
  * No `tr`/`trf`/`ApiError::tr` is used in a static, const or OnceLock initialiser.
  * Every text cache is keyed by language or rendered when read:
    * the disks snapshot;
    * the `VolumeCache`;
    * scan trees (`localize`);
    * the stored scan error (`message`);
    * categories (with B1);
    * health `names_lang`;
    * the index `last_status` and the root errors of the worker's `State.errors`.
  * Already keyed: `DesktopIndex` and the cleanup list cache.
  * Language-neutral: `lsblk` and boot entries (raw data), `cpuinfo` ("Processor" is the same word in both
    languages), and the GPU names.
* **Missed sites (check 3):** `extract_rs.py list` on all 21 owned files shows only the 8 `own_names` pair literals
  and 3 `new:?` technical literals (an error code and two json keys). The only real miss is M1 (`--all` scan). The
  allow rules are narrow (n1).
* **English (check 4):**
  * I read all 138 en rows (sysmon 70, storage 23, index 44, disks 1).
  * They use ’ throughout, with no ASCII apostrophe and no straight double quote.
  * They are sentence case, en-GB ("colour management", "any more"), and follow the glossary (Recycle Bin,
    Efficiency mode, Startup apps, Task Manager).
  * App names and C9 items are kept (Alles, Win11 Shell, GNOME, Flatpak, Snap, GVfs, PipeWire, `dm:`/`size:`/`ext:`/
    `parent:` examples).
  * The text.json English parts are fine.
  * Only m1 remains.
* **Behaviour (check 5):**
  * Codes are unchanged: `cancelled`, `bad_query`, `bad_regex`, `index_unavailable`, `index_restarting`,
    `confirm_required`, `unknown_method`, `unavailable`, `failed`, `busy`, `not_found`, `protected`, `unsupported`,
    `launch_failed`.
  * These are the codes `vendor/ui/js/apps/{alles,taskmgr,disks}` match on. The UI matches no message text.
  * `storage.scanDone`/`scanResult` keep `errorCode`.
  * The envelopes keep `code` and `message` and add key + params (C2).
  * Oracles: untouched and green.
* **Extras (check 6):**
  * Extra 1: done and tested, except B1.
  * Extra 2: done, and English no longer repeats the heading.
  * Extra 3: done. The 500 ms watcher sends the status again on a switch (also when Alles gave up), and the worker's
    own 500 ms status is rendered by the host.
  * Extra 4: done (28 rows).
  * Extras 5 and 6: see M2.
* **Ownership (check 8):**
  * `git status --short` and `git diff --stat` list 22 changed files plus the 5 new paths, all owned.
  * No frozen file was touched, and `disks/reference-tests.json` is untouched.
  * rustfmt: `--check` is clean on the 4 module trees now and on a d666df54 `git archive` copy, so the formatting
    pass added no noise outside changed code.
  * The build snapshot `tests/build/r2b-debug` has the same 12 executable names as r1-debug. SHA256SUMS checks out,
    the files are byte-identical to `target/debug`, and they were built at 19:03, after the last source or row edit
    (18:56).

## Done-when, run by me (from W's root)

| Check | Result |
|---|---|
| `drift_rust.py --paths <owned files> --warnings` | 13450 frozen literals in 116 files; 4770 present, 242 mapped or listed; 165 rows; 221 new; **0 errors, 0 warnings** |
| `drift_rust.py --warnings` (incl. ratchet) | 13208 present, 242 mapped; 165 rows; 395 new {technical 381, bilingual-pair 6, log 4, user-visible-dutch 3, user-visible-english 1: all GE1b's}; **0 errors, 0 warnings** |
| `check_rust.py --warnings` (node on PATH) | 24 stems, 165 rows; **0 errors, 1 warning** (`[glossary] disks.unavailableBody … lacks 'Disk Management'`, intended) |
| `check.py` | **0 errors**, 53 warnings (52 glossary, 1 rust) |
| `heavy.sh -- cargo check --offline --locked -j2 --all-targets` | rc 0; lib 28, lib test 26, win11-launch 2, launch test 1 (the baseline counts); 7 warnings in owned files, all on pre-existing items (`Run.err` ×2, `last_changed`, `counts`, `state_name`, `CString`, `NOSHOW`) |
| `heavy.sh --min-mb 2048 -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | rc 0; lib **287 passed, 8 ignored** (271 + 16, which equals the 16 `#[test]` added in the diff); win11-launch 5 passed, 1 ignored; oracles 16/16 (`oracle_tests::*` 8, the 7 python `disks::tests`, `reference_contracts`); `every_converted_site_renders_its_3b898e6_dutch` ok; the 16 new R2-B tests ok; `guard-calls.log` 0 bytes |
| `heavy.sh -- run-tests.sh --lib -- unit_friendly_matches --nocapture` | nl 0.539 µs at 3b898e6, 1.497 µs now; en 0.015 µs at 3b898e6, 1.458 µs now (58 units) |
| `ge1b-perf.py` r1 against r2b, `ge1a-disks-race.py` / `r2b-disks-race.py` | not run (M2) |
