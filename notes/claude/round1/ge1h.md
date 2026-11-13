# GE1h: scenarios, F24 guards and heavy.sh for the private GNOME harness (2026-11-05)

G = `/home/robin/Documents/Codex/2026-11-01/chat/outputs/win11-gnome`, branch `claude/gnome-english-update`. Nothing
was committed. No product file was touched. Nothing ran outside the sandboxes: every GNOME run went through
`tools/heavy.sh --min-mb 2560`, and the guard self-test starts no GUI process.

## 1. Files

| Path | Change |
|---|---|
| `tests/gnome/harness.py` | 1,527 → 1,928 lines (+527/−85 per `git diff --stat`); details below |
| `tests/gnome/lib/__init__.py`, `tests/gnome/lib/scenario.py` | new; `scenario.py` is 735 lines: the `Helper` (`h`), the JS snippets and the scenario runner |
| `tests/gnome/scenarios/example.py` | new; the example scenario (126 lines) |
| `tests/gnome/README.md` | 159 → 234 lines: heavy.sh usage, the new options, the F24 guard list and a "Scenarios" section with the whole `h` API |
| generated: `tests/gnome/baseline/guard-selftest.json` | rewritten by the self-test, which now has 110 cases (it had 56) |
| generated: `tests/gnome/scenarios/example.out/` | the last example run: `results.json` (134 kB), 43 PNGs and `logs/` (shell, harness, install, memory, 7 app logs); 2.2 MB |

My files did not change underneath me during the step (sha256 checked before and after the runs). Other steps
edited `extension/`, `scripts/`, `vendor/ui` and `tools/i18n` at the same time, as planned.

## 2. What the harness can do now

* **`--build-dir DIR`** chooses which 12 binaries are installed. The default is `tests/build/3b898e6-debug`.
  * The directory must be inside G (not in `~/.local`) and hold all 12 executables.
  * This is checked in `launch` before any lock is taken, and again inside the sandbox.
  * `WIN11_BUILD_DIR` must equal `--build-dir` [`build-dir-mismatch`].
* **`--source worktree|<rev>`** (not requested; I added it) chooses the tree whose `install.py`, `vendor/`,
  icons and `extension/` are installed.
  * A revision is exported with `git archive` into `run/src`. That takes 7.9 s and does not write to the repository.
  * The Settings page titles and the seeded defaults come from the same tree.
  * Why it is needed: GE1a and GE1c were editing `vendor/ui`, `extension/` and `scripts/` during my runs, so a
    worktree install would have mixed half-done edits into the baseline.
* **`--scenario FILE`** runs a `.py` file inside G with `def run(h)`.
  * It runs inside the sandbox after the shell is up, the extension is active, panel, desktop and the preloaded
    Start are ready, and GNOME's own notices are dismissed.
  * It replaces the built-in pass; add `--surfaces` to run both.
  * Output goes to `<scenario dir>/<stem>.out/`, or to `--out`. `--out` must be inside `tests/gnome/` and
    outside `run/`.
  * Progress is also written to `run/logs/scenario-results.json` after every step.
  * The exit code is 1 when the scenario raises (including through `h.require`); a failed `h.check` does not
    change it.
* **`h` (`lib/scenario.py`)**, every call recorded in `results.json` → `scenario.steps`:
  * **`open(spec)`:**
    * `panel` and `desktop`;
    * `start` (the extension's `_toggleStart`);
    * `explorer` (`tabs=2` clicks the real "Nieuw tabblad" button);
    * `settings` and `settings:<page>`;
    * `taskmgr`, `alles`, `disks`, `defaultapps`, `installer`.
  * **Window and input helpers:** `close`, `new_tab`, and `click`. `click` uses a real Clutter virtual pointer
    by default, or a DOM `click()`, and records the pointer events the page saw.
  * **Evidence helpers:**
    * `screenshot(name, surface|area)`;
    * `page_text`: text plus placeholders, aria-labels and Dutch-word hits;
    * `wait_text` and `wait_lang`.
  * **`set_language(lang, via)`** records the input path used and the seconds until each surface's `<html lang>`
    changed:
    * `dropdown`: the real Taal en regio control, with real pointer clicks and a recorded DOM fallback;
    * `rename`: lock, temp file and rename, as the store does;
    * `rewrite`: in place, same inode.
  * **`window_titles()`** (Meta titles through Eval) and **`app_names(ids)`** (`Shell.AppSystem lookup_app(id).get_name()`, the 10 own ids by default).
  * **`extension_labels()`** walks the extension's actor tree, so it does not depend on GE1c's field names. It
    returns the fallback bar's visibility, its labels and accessible names, the clock text, the Ctrl+Alt+Tab
    group and the Dutch-word hits.
  * **`kill_panel()`** signals only a verified descendant and returns the fallback bar's labels and a screenshot.
    **`restart_panel()`** starts the panel again.
  * **`hidden_start_first_frames(lang, via)`**:
    1. arms a recorder in the hidden Start: per animation frame after `win11:start-show`, it stores `<html lang>`,
       the `.sh-start` text and the search placeholder;
    2. switches without touching Start;
    3. opens Start and takes up to 16 back-to-back screenshots of its area (about 27 ms apart).
  * **Recording:** `check(name, ok, known=…)`, `require`, `record`, `log`.
  * **Low level:** `eval`, `js`, `until`, `settings_file`, `notifications`.
* **F24 guards.** Each condition has its own refusal code and is refused even when set to an empty or a
  private-looking value:
  * `NO_AT_BRIDGE` ≠ `1` and `GTK_A11Y` ≠ `none` (empty included);
  * HOME is the real home (also written as `/.`);
  * `XDG_DATA_DIRS` has an entry in `~/.local/share` [`xdg-data-dirs-home`];
  * these variables are set: `WIN11_REFERENCE_SETTINGS` [`reference-settings-set`], `WIN11_REFERENCE_INSTANCE`
    [`reference-instance-set`], `WIN11_GNOME_CONFIG_DIR` [`gnome-config-dir-set`], `AT_SPI_BUS_ADDRESS`
    [`at-spi-bus-set`], `XDG_SESSION_ID` [`xdg-session-id-set`];
  * F24's last item [`extension-copy-not-private`]. It is checked on the installed copy, before
    `--patch-extension` patches anything, and on the `path` the shell reports through Eval.
  * Added to the generic list as well: `XDG_SEAT`, `XDG_VTNR`, `XDG_SEAT_PATH`, `XDG_SESSION_PATH` and
    `GNOME_SETUP_DISPLAY`.
* **Heavy lock (C8).** `launch run` recognises `tools/heavy.sh` as an ancestor: its script path in the argv,
  an open fd on the lock file, and the lock really held (a non-blocking attempt fails). Before, only `flock(1)`
  was recognised, so under heavy.sh the harness would have waited an hour for a lock its own parent held.
  `flock` ancestors still work. With no ancestor, the run is refused (exit 4) and the heavy.sh command is
  printed; `--own-lock` restores GE0c's self-locking. `WIN11_HEAVY_LOCK` is honoured as in heavy.sh. The
  minimum MemAvailable went from 2 GB to 2560 MB.

## 3. Verification

```bash
cd $G
python3 -B tests/gnome/harness.py guard-selftest          # 7.6 s, no GUI
tools/heavy.sh --min-mb 2560 --tag ge1h -- python3 -B tests/gnome/harness.py launch run --lang nl \
    --source 3b898e6 --scenario tests/gnome/scenarios/example.py
```

**Guard self-test: 110/110 passed, exit 0** (`baseline/guard-selftest.json`, 14:51).

| Layer | Cases |
|---|---|
| in-process | 54 |
| process (outside the sandbox) | 4 |
| sandbox (real bwrap) | 47 |
| lock | 5 |

* **F24:** 43 cases, plus one composite case per layer that requires all 9 F24 codes at once.
* **Extension copy:** the private copy is accepted; the live, repo and system copies are refused.
* **Lock decisions,** using a scratch lock in `tests/logs/` (so the real lock was not touched):

  | Situation | Result |
  |---|---|
  | No ancestor | refused, exit 4 |
  | `flock` ancestor on the lock | accepted |
  | `flock` ancestor on another file | refused |
  | `heavy.sh` ancestor | accepted |
  | `--own-lock` | lock taken |
* **The caller's own environment is refused.** Besides the GE0c codes, it now also gives `xdg-data-dirs-home`:
  this agent's environment has `~/.local/share` in `XDG_DATA_DIRS`, which is exactly the F24 gap.

**Example scenario.** There were three runs.

| Run | Result | Cause or change |
|---|---|---|
| 1 | Stopped after 9.7 s of scenario (exit 1, clean shutdown) | A bug in `h.open`: it applied the Explorer tab logic to a reused Settings window |
| 2 | Exit 0 | After the fix, and after creating the virtual pointer before the first click (Wayland clients bind `wl_pointer` late, so run 1's first real click was lost and fell back to a DOM click) |
| 3 | **Final** | Extended with an in-place rewrite to English while Start is hidden |

**Final run (15:00:27-15:03:04): exit 0.**

* **Time:** 155.0 s wall, of which 84.8 s is the install under strace and 32.3 s is the scenario.
* **Install:** heavy lock "held by an ancestor heavy.sh"; 87,841 mutating syscall paths, 0 outside `run/`;
  12/12 binaries byte-identical to `tests/build/3b898e6-debug`; 9/9 extension files identical to `3b898e6`.
* **Shell:** GNOME Shell 50.1, session mode `user`, `unsafe_mode` true; the extension is active and its copy
  is the private one.
* **Memory:**
  * peak PSS 1,141 MiB for the whole sandbox, 1,055 MiB without the harness;
  * 34 processes;
  * lowest MemAvailable 3,628 MiB;
  * no memory abort; 72 samples.
* **Cleanup:** no leftover processes; `pgrep` on the host afterwards found nothing, and the real `/tmp` holds no
  `win11` entry.
* **Stub calls:** `wpctl` 40, `pw-dump` 15, `systemctl` 4, `apt` 1. Nothing was executed.
* **Checks:** 23 of 29 passed. The 6 that failed are all marked `known`, and none is unexpected:

| Check | Result at 3b898e6 |
|---|---|
| Real input | The pointer clicked the "Nieuw tabblad" button, the dropdown button and the "English (United Kingdom)" option. Each time the page saw trusted `pointerdown`, `pointerup` and `click` at the expected client coordinates, and no fallback was needed. |
| Dropdown nl→en | Settings wrote `language=en` by rename (new inode). `<html lang>` was polled one surface at a time, so each value is an upper bound with about 0.1 s resolution: panel 0.72 s, desktop 0.82, Start 0.92, Settings 1.03, Explorer 1.08, Settings page title 1.28 s. |
| **F1**, Start opened after the dropdown switch | **Fails (known).** The headings "Vastgemaakt", "Alle apps" and "Aanbevolen" stay Dutch, the search placeholder is "Type here to search", and the app names are English (`04-en-start-after-switch.png`). |
| Rename en→nl with Start hidden | panel 0.10 s, desktop 0.20, Settings 0.26, Explorer 0.36, title 0.56. Start opened in Dutch from its first recorded frame: 16 screenshots over 0.42 s, 31 DOM frames. |
| **F1 (F28 path)**, in-place rewrite nl→en with Start hidden | Same inode; panel 0.10 s, title 0.52 s. **Fails (known): Start opens with the stale Dutch headings from its first DOM frame to its last** (32 frames, none clean, `firstDomFrameInTargetLanguageMs: null`; `hidden-start-en-frame-15.png`). So the hidden preloaded Start shows F1 too. GE1a must fix both paths. |
| **F2** (C4) | **Fails in nl (known).** The Meta titles are "Settings", "File Explorer", "Win11 Taskbar" and "Win11 Desktop" in both languages. The en checks pass only because the titles are English anyway. |
| **C6** | **Fails (known).** `app_names` is identical in nl and en: "Win11 Instellingen", "Win11 Verkenner", "Win11 Taakbeheer", "Alles", … There is no runtime rewrite at the baseline. |
| **GE1c** (extension labels) | **Fails (known).** In en mode the fallback bar still has "Zoeken", "Taakweergave", "Netwerk, geluid en snelle instellingen" and the Ctrl+Alt+Tab group "Win11-taakbalk", and the clock reads "14:58\n05-11-2026". The bar appeared 0.11 s after `kill_panel`. |
| Tour in en | Taakbeheer, Alles, Schijven, Standaard-apps and the installer each reported `lang=en` with 0 Dutch-word hits. |

Earlier, with the mock: before each GNOME run I ran the example against a fake Session in my scratchpad (no
processes) to catch Python errors. This is not part of the deliverable.

## 4. Deviations from plan and contracts

1. **Ownership conflict in the plan.** `plan.md`'s round-1 table lists "`tests/gnome/harness.py` core" under
   "nobody, read-only in round 1", and GE1h is not in that table. My task gives me `harness.py`. I followed the
   task. Please add GE1h to the table.
2. **Files written outside the listed allowlist, all of them run output:**
   * `tests/gnome/baseline/guard-selftest.json`, rewritten by the self-test as it always is;
   * `tests/gnome/scenarios/example.out/`;
   * `tests/logs/heavy-ge1h-*.time` and lines in `heavy-events.log` (gitignored);
   * a scratch lock in `tests/logs/` during the self-test, which was deleted.
3. **Behaviour change.** A bare `launch run` without heavy.sh or flock now refuses instead of taking the lock
   itself, and the memory floor went from 2 GB to 2560 MB. Both bring the harness in line with C8.
4. **The default build changed** from `target/debug` to `tests/build/3b898e6-debug`, as the task asked. A step
   that tests its own Rust must pass `--build-dir target/debug`.
5. **`--source` was added** (it was not requested), for the reason given in section 2.
6. Without `--out`, the built-in pass still writes to `baseline/<lang>/` and overwrites the committed baseline.
   This is the GE0c behaviour, unchanged; it is now documented.

## 5. Left, and decisions for the coordinator

* **F1 now covers two paths:** Start open during the switch, and Start hidden and preloaded during the switch
  (the F28 path). The English search placeholder also differs between a fresh start and a live switch ("Type
  here to search"). Owner: GE1a. The example already has the checks, with `known='F1'`.
* **Each step should copy `example.py`** to `scenarios/<step>-*.py` and turn its known failures into
  `h.require` once its fix lands:
  * GE1a: F1;
  * GE1b: F2/C4 titles and C6 names;
  * GE1c: the extension labels, the clock format and the Ctrl+Alt+Tab group.
* **To commit, or not:** whether to commit `scenarios/example.out/` (2.2 MB, 43 PNGs) or only its
  `results.json`. The `run/` directory stays excluded.
* **Not covered yet:**
  * the Ctrl+Alt+Tab popup itself (only the group name is read);
  * notifications mirrored by the extension;
  * keyboard input;
  * scale 2 or a second monitor;
  * Explorer tab transfer and the picker during a switch;
  * a pixel-diff tool.

  These are left for round 3 or for the steps that need them.
* **Timing figures** from `set_language` come from polling one surface at a time (about 0.1 s per surface).
  Treat them as upper bounds. For finer numbers, add a recorder inside the page, like the one in
  `hidden_start_first_frames`.
