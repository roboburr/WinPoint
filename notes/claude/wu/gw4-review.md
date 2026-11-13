# GW4 review: Updategeschiedenis, Geavanceerde opties, D2 (adversarial, 2026-11-05)

Reviewer: independent agent; I did not write GW4. Worktree `.wt/gw4` (branch `wu/gw4`, HEAD f01710cc, GW4's changes
uncommitted). I read design §3.6, §4.1–4.5, §4.11, §4.12, §9.2, §10, plan.md D-G25..D-G42, contracts C10,
`notes/claude/wu/gw4.md`, every changed file, and GW2's `update.rs`, `guard.rs`, `store.rs`, `model.rs` and `pk.rs` where
GW4 depends on them. I edited no product file. My scratch output is in `.rt/review-gw4/`: logs, strace, dumps, a paging
simulation and the screenshots I took (moved there from `tests/update-ui/shots/*-rv*`). GW4's own shots are untouched.

## Verdict: fix-needed

The safety check is clean. I found no path to the real system bus, the live session, the network or the real dconf,
and no tool runs from outside the tool table. strace confirms this. Every done-when check is green when I re-run it.
One major defect remains: a stale `job.json` gives the wrong history source. There are also several minor defects in
history paging, release-upgrade detection and the page code.

## Checks I ran (from W's root)

| Check | Command | Result |
|---|---|---|
| full unit suite | `tools/heavy.sh --min-mb 2048 --tag gw4-rv-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | rc 0; lib **378 passed, 0 failed, 16 ignored**; win11-launch 16 passed, 1 ignored; `guard-calls.log` **0 bytes**; tmp entries left 0. One run, no hang (the run-1 hang GW4 reports is not reproduced; its log `.rt/gw4/test-full-run1-hang.log` shows no `services::update` test involved) |
| GW4 tests under strace | `tools/heavy.sh --min-mb 2048 --tag gw4-rv-strace -- tests/rust-sandbox/run-tests.sh --lib --config "target.x86_64-unknown-linux-gnu.runner=['/usr/bin/strace','-f','-qq','-o','<W>/.rt/review-gw4/strace.log','-e','trace=execve,connect,openat,unlink,rename,mkdir']" -- services::update::history services::update::options` | 12 passed. **execve**: the test binary + 25 stubs, all `…/w11u-*/tools/{apt-config,snap}`, each with a 4-variable environment; 0 other programs. **connect**: 1, to the sandbox's private session bus `/tmp/w11-ge0b-*/dbus-*` (GVfs, loaded by the keyfile backend, see N1). No `dconf`, `/run/user`, `system_bus_socket` or real `~/.config` path anywhere. All 314 write opens are under `/tmp/w11-ge0b-*` or `/dev/null`. No read of the real `/var/log`, `/proc/locks` or `/proc/*/cmdline` |
| cargo check | `tools/heavy.sh --tag gw4-rv-check -- bash -c 'cd host && cargo check --offline --locked -j2 --all-targets --message-format=short'` | rc 0; warnings lib 28, lib test 26, launch 2, launch test 1 (= baseline); **0** in `services/update` |
| drift | `python3 -B tools/i18n/drift_rust.py --warnings` | **0 errors**, 2 warnings (`gschemas.compiled`, `usr/share/glib-2.0/schemas`: GLib file names) |
| check_rust | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` | **0 errors**, 72 warnings (64 unused GW5–GW9 rows etc., none new) |
| check.py | `PATH=… python3 -B tools/i18n/check.py` | **0 errors**, 124 warnings (glossary 52, rust 72) |
| ratchet | `python3 -B tools/i18n/ratchet.py` | `ui: ok (89 allowed occurrences, 0 still to convert)` |
| dict diff | `PATH=… python3 -B tools/i18n/check_ui.py --dict-diff tests/i18n/dict-nl-3b898e6.tsv` | 2925 → 3164 rows; 9 changed, 20 removed, 259 added; **0 not in BASELINE-CHANGES.ui, 0 stale** |
| node | `node --check --input-type=module < F` (update-history.js, update-advanced.js, mock/update.js) | 3 × ok |
| manifest | `python3 -B tools/ui-manifest.py --check`; `python3 -B scripts/verify-ui.py` | up to date (181 files); OK, 181 files |
| i18n unit tests | `python3 -B -m unittest tests/i18n/test_update_mock_rows.py`, `… tests/i18n/test_ge0a.py` | 5 OK (1 skipped); 21 OK (2 skipped) |
| nl dump | `tools/heavy.sh --tag gw4-rv-dump -- tests/i18n/run.sh --notes .rt/review-gw4/notes-nl.json` | "nl dump equals baseline-nl.txt (24585 lines)"; console note only `taskmgr-dialog.restart-shell` (not GW4) |
| en dump | `… tests/i18n/run.sh --lang en --notes …` | "en dump equals baseline-en.txt (24582 lines)" |
| baselines vs HEAD | block compare of `git show HEAD:tests/i18n/baseline-{nl,en}.txt` against the recorded files | nl and en: changed exactly `settings.update-history`, `settings.update-advanced` (only added lines + GW3's placeholder line `Er is nog geen updategeschiedenis.` / `There’s no update history yet.` gone from the default page; still in S02), **14 new**, **0 removed** |
| existing Dutch | `git show HEAD:` of both pages vs now | GW3's `hEmpty`, `advUpdater`, `open`, `updaterTitle`, `updaterBody` byte-identical in nl and en |
| CSS scope | `git diff -U0 vendor/ui/css/settings.css` | one hunk at 816–829, inside "Update rows" (781–830) |
| fixtures | `make.py` copied to `.rt/review-gw4/mk/` and run with `python3 -I -B` | the regenerated `var/` equals `tests/update/fixtures/history/var` byte for byte |
| screenshots | `SUFFIX=-rv tools/heavy.sh --tag gw4-rv-shots -- tests/update-ui/shoot.sh <16 ids>`; `LANGS=en SIZE=800x1400 SUFFIX=-rv-narrow … <5 ids>`; `THEME=dark LANGS=nl SUFFIX=-rv-dark … <2 ids>` | 32 + 5 + 2 shots, 0 failed; reviewed below |
| ownership | `git status --short`, `git diff --stat` | only owned files (12 modified + `notes/claude/wu/gw4.md` + `tests/update/fixtures/`), 3175+/54−; unchanged by my runs |

## Findings

### M1 (major): a stale unfinished `job.json` gives every later PackageKit run and release upgrade the source "via Win11 Shell"

* Evidence: `history.rs:117-121` adds `(startedAt, now)` for any `job.json` without `finishedAt`. It does not check
  `bootId` or whether the owner is alive. GW2 never finishes such a file when it comes from another boot:
  `model.rs:593-599` returns `Reattach::Nothing`, and `pk.rs:1406-1408` then returns. The file stays until the next
  own job overwrites it. The power guard blocks a reboot only while the job commits (`guard.rs:57-62`).
* Scenario: Robin clicks "Alles downloaden en installeren" and reboots during the download. From then on, every
  `packagekit role='install-packages'` run with `Requested-By: robin (1000)` is matched by `ours()`, because the
  window grows to "now". These are GNOME Software installs and updates. Every release upgrade started from Ubuntu's
  own updater is matched too (pkexec sets Requested-By, and `history.rs:368` checks only uid + window). All of them
  read "via Win11 Shell" instead of "via een app" or nothing.
* Fix: use an unfinished `job.json` only when `job["bootId"] == store::boot_id(paths)` and
  `store::owner_alive(&job["owner"])`, the same filter as `update.rs:601-606`. Otherwise end its window at
  `store.modified_ms(JOB)` or drop it. Add a unit test with an other-boot `job.json`.

### m1 (minor): "Meer laden" loses entries when a run has 1000 or more entries, or when two runs end in the same second

* Evidence: `history.rs:351` sets `date = base + 999 - i.min(999)`, so entry 999 and every later entry of one run
  share the date `base`. Two runs that end in the same second also share dates. `page()` (`history.rs:387`) then
  continues with the strict `e.date < before`.
* Simulation of the same rules (`.rt/review-gw4/page_sim.py`):
  * a 1200-package run at limit 200 lists **1000 of 1200**;
  * two two-package runs in one second at limit 1 list `['a0', 'a1']` of `['a0', 'b0', 'a1', 'b1']`.
* When this happens: unattended-upgrades' MinimalSteps writes many short runs; the first full upgrade after an install
  or point release can have about 1000 packages.
* Fix: make the cursor unique. Either add an opaque `cursor` (run index + entry index) next to `date`, or let `before`
  be `{date, seq}`. Keep `date` for display only.

### m2 (minor): a partial upgrade is shown as an Ubuntu release upgrade, and its updates disappear from "Updates"

* Evidence: `/usr/lib/ubuntu-release-upgrader/do-partial-upgrade:98-113` (read only) runs `setup_logging` and
  `DistUpgradeController`, the same code as a release upgrade. So `DistUpgradeCache._initAptLog` also points
  `Dir::Log` at `/var/log/dist-upgrade` for Software Updater's "partial upgrade". `history.rs:355-370` turns every such
  file into one `releases` entry: "Ubuntu 26.04" (the current release) or a bare "Ubuntu". The individual packages are
  never listed.
* Fix: count a dist-upgrade log as a release upgrade only when the old and new versions of an `UPGRADER_PACKAGES` or
  `base-files` entry name different releases (`1:25.10.9 → 1:26.04.14`). Otherwise list its runs like apt runs
  (groups updates/software, source `app`). Add a fixture.

### m3 (minor): after 90 days our own updates change from "via Win11 Shell" to "via een app"

* Evidence: `store.rs:25-26, 103-111` drops `jobs.jsonl` lines older than 90 days. `history.rs` reads up to
  `.4.gz` (4–5 months), and "Meer laden" reaches those entries. A PackageKit run of our uid without a window is
  labelled `app` (`history.rs:323-325`).
* Fix: give a PackageKit run of our uid older than the `jobs.jsonl` horizon `source: null` rather than `app`. The
  alternative is to ask GW2 or round 3 to keep `jobs.jsonl` as long as the rotated logs (≈ 150 days).

### m4 (minor): `history.log` over 8 MiB loses its newest runs

* Evidence: `history.rs:179` uses `File::take(MAX_READ)`, which reads the first 8 MiB, the oldest part. apt appends
  at the end.
* Fix: for a plain file larger than `MAX_READ`, seek to `len − MAX_READ` and start at the next `Start-Date:`.

### m5 (minor): Geavanceerde opties stays empty until both programs have answered

* Evidence: `update-advanced.js:78` awaits `update.options` before it builds any card. That includes the two
  settings toggles and the Software Updater row, which do not need it. `options.rs:51-53` runs `apt-config` and `snap`
  one after the other, with 5 s each (`options.rs:43, 177-187`), and `bridge.call` has no timeout.
* Effect: a slow snapd leaves the page blank for up to 10 s. Windows opens this page at once.
* Fix: render the page first and fill the security, snaps and D2 rows when `update.options` answers. The two programs
  could also run in parallel with a shorter timeout.

### m6 (minor): a lost mirror write gives no feedback, and the owner replies late

* Evidence, the page: `update-advanced.js:126-129` shows nothing when no `update.reply` arrives within 5 s (for
  example, the owner has gone). The toggle silently flips back.
* Evidence, the owner: `update.rs` `on_request` runs `set_auto_open`, which calls `view()` (`options.rs:91`) and
  starts both programs again before the reply is sent. The forwarding path never uses that view.
* Fix: on timeout, show `showError` or a notice. In the forwarded path, reply right after the write, for example by
  splitting `set_auto_open` into write + view.

### m7 (minor, wording): "om de 1 dag" / "every 1 day"

* Evidence: `update-advanced.js:29, 50` (and dump S03e: "Ubuntu opent de Software-updater om de 1 dag …").
* Problem: in Dutch, "om de dag" reads as "every other day". "every 1 day" is not Windows English either. GW4
  followed §3.6's plural literally, so this is a design wording issue.
* Fix (GW4's decision 3): one = "Ubuntu opent de Software-updater elke dag als er gewone updates zijn." / "Ubuntu
  opens Software Updater every day when regular updates are available." This is one nl row more in
  `BASELINE-ADDITIONS.ui`.

### m8 (minor, i18n rule): the setAutoOpen errors use a sysmon catalogue row

* Evidence: `options.rs:67` uses `ApiError::tr(…, "sysmon.startup.saveFailed", …)`. The rule is "host texts from
  `update.tsv`".
* GW4's reason is correct: `drift_rust.py:19-20, 843` require a `key:` line in a `BASELINE-CHANGES.rust*` file for a
  `#@ new` row, and GW4 owns none.
* Risk: a later change to sysmon's text silently changes this update error.
* Fix: the coordinator grants `tests/i18n/BASELINE-CHANGES.rust-gw4`, then GW4 adds `update.options.err.autoOpen` with
  `extract_rs.py add` (GW4's decision 1).

### m9 (minor): the Snap Store row says "updated automatically" while refreshes are held

* Evidence: `update-advanced.js:106-107` never reads `snaps.hold`, which `options.rs:154` returns. A user who ran
  `snap refresh --hold` still reads "Worden automatisch bijgewerkt door Ubuntu".
* Fix: this needs a design text, for example "Automatisch bijwerken is gepauzeerd tot {when}" (a decision for round 3),
  or drop `hold` until then.

### Notes

* **N1, test mode still touches the session bus.** The keyfile backend loads GVfs, which connects to
  `$DBUS_SESSION_BUS_ADDRESS`. strace shows one connect, to the sandbox's private bus. Test mode never touches dconf,
  but gw4.md's "nothing at all" does not cover the session bus. Every sanctioned sandbox keeps that bus private. The
  harness and `run.sh` could add `GIO_USE_VFS=local` (coordinator; not GW4's files).
* **N2, absolute production paths bypass PATH stubs.** In production the tool table runs `/usr/bin/snap` and
  `/usr/bin/apt-config` by absolute path, so the harness's PATH stubs never catch them. A GNOME-harness scenario that
  opens Geavanceerde opties without `WIN11_UPDATE_TOOL_DIR` would run the real snap against `/run/snapd.socket`, which
  is still visible there (D-G42). GW8 and GX1 must set the seams or get D-G42's masking first.
* **N3, test mode without a fixture root reads real files.** It reads the real `/var/log/apt` and
  `/var/log/dist-upgrade` (`Paths::rooted(None)`). A harness shot without the fixture root would show Robin's real
  package history (GW8).
* **N4, fixture schema checks.** In `options.rs:102`, `from_directory(dir, None, true)` trusts the fixture's
  `gschemas.compiled` unchecked; `false` would be safer. `options.rs:110` assumes the schema has a path. A
  relocatable schema of that id would abort in `g_settings_new_full`; check `schema.path()` first. Defensive only.
* **N5, paths.rs is two hunks.** The change is a struct field and its initialiser, both at the end and both marked
  `// GW4`. That is the minimum for a struct and is acceptable.
* **N6, accepted deviations.** Deviation 1 (`/var/log/dist-upgrade`) is correct: I verified it in
  `DistUpgradeCache.py:220-227` and `DistUpgradeMain.py:74-86`. Deviation 2 (no programs while the upgrader runs) is
  defensible; whether to keep it is GW4's decision 2. Deviation 8 (dry-run shows "Niet beschikbaar") follows §4.11's
  stricter rule. Deviation 9 (keyfile in the fixture root, not `$HOME`) meets the safety aim, but GW8 must copy the
  schema into its fixture root.
* **N7, the count after "Meer laden".** The group count "(n)" counts loaded entries, so it grows after "Meer laden".
  `paint()` also rebuilds the page, so keyboard focus leaves the clicked button.
* **N8, Windows 24H2 gaps that need new keys (design level).** There is no "Aanvullende opties" / "Additional options"
  header (GW4 decision 4). Windows' "Notify me when a restart is required…" row has a description that §3.6 lacks.
  Update history has no "Gerelateerde instellingen" (Herstel).

## Safety analysis (CHECK 1)

* **Seams.** GW4 adds no seam. Its two `Paths` fields are `None` in production and inside the fixture root in test
  mode. In test mode without a fixture root, `notifier()` returns `None` (`options.rs:96-98`): there is no schema
  source and no backend, so it can never reach the default dconf backend. A failed seam answers `test_seam` before
  GW4's code runs (`update.rs` `call`).
* **Programs.** Both programs start only through `Tools::command`. That is the table path, with a cleared environment
  plus LANG, PATH and dead buses in test mode; strace shows 4 variables. They run only in a non-dry `settings` host
  and never while the upgrader runs. The exec-path grep test passes, and the GW4 files contain no `Command::new`,
  absolute path or `env::var`.
* **Network and buses.** There is no network code and no bus call. The one connect is GVfs to the private session bus
  (N1).
* **Writes.** In tests the only writes are the keyfile inside the fixture root, `settings.json` in the fixture config
  and the stub log. `gio::Settings::sync()` is skipped in test mode, so no default backend is created.
* **Live risk outside GW4's tests.** The only one is N2, which belongs to GW8/GX1.

## Conformance to §10 GW4 and §3.6/§4.3/§4.12 (CHECK 2)

| Requirement | Status |
|---|---|
| history from history.log + .1.gz–.4.gz through Paths; ≤ 200 / 90 days; more | done (+ `.1`, + dist-upgrade logs); paging defects m1, m4 |
| groups Updates / Ubuntu-versies / Software; ok/failed | done; partial upgrades misfiled (m2) |
| sources ubuntu / us (jobs.jsonl window + uid) / app / terminal | done; stale window M1, 90-day horizon m3 |
| parser fixtures: gz rotation, truncated entry, release upgrade, failed dpkg, non-UTF-8 | done (6 tests, all pass, fixtures reproducible) |
| options: unattended via apt-config, snaps via snap stub, autoOpen {supported,on,days} | done (+ `hold`, unused, m9) |
| setAutoOpen: off 36500 + autoOpenPrevious; on restores or default 7; regular updates only | done (default = `reset`, which honours vendor overrides) |
| tests: stubs from TOOL_DIR, keyfile backend, never real dconf | done (keyfile in the fixture root, N6) |
| toggles persist through settings.set | done (`the_two_page_toggles_persist_through_settings_set`) |
| pages with §3.6 texts nl+en via makeT; D2 names "gewone updates" and the security exception; plural | done (wording m7) |
| Software Updater row kept | done, byte-identical texts |
| mock states for every state as surfaces, in both dumps | done (14 surfaces, S01–S03h) |
| host errors from update.tsv | not met for setAutoOpen (m8, blocked by ownership) |

## Language (CHECK 4)

* No Dutch literal in GW4's Rust (drift 0 errors, ratchet ok).
* The nl and en texts equal §3.6, apart from GW4's documented split of `advAutoOpenDesc` and the zero form.
* en-GB dates (25/10/2026) and ’ are correct. Windows wording is used for the metered and restart rows and for
  "Successfully installed on" / "Failed to install on".
* The page branches on error codes (`unknown_method`), never on message text.
* Existing Dutch is unchanged.
* The only wording issue is m7.

## Screenshot review (CHECK 5, own shots in `.rt/review-gw4/shots/`)

* **History** (nl, en, dark, en 800 px):
  * expanders "Updates (12)" / "Ubuntu-versies (2)" / "Software (3)" without icons, with chevrons; two-line entries
    (body, then the caption date line), dividers, text aligned at 16 px, as in 24H2;
  * failed entries are worded like Windows, with no colour (also like Windows);
  * dark mode is correct;
  * at 800 px, nothing is clipped; only the shared breadcrumb is cut ("Update hi…", as on GW3's pages);
  * "Meer laden" is ours (Windows loads everything).
* **Advanced options**:
  * toggle cards without icons in Windows' order (metered, then restart notification), "Aan/Uit" left of the switch;
  * D2 wraps to two lines at 1280 px;
  * icon rows after a group gap, standing in for Windows' "Additional options" header (N8);
  * at 800 px the controls wrap under the title, left-aligned like WinUI's vertical SettingsCard;
  * S03c hides D2; S03h shows "Not available" for the read-only rows;
  * no overflow in nl or en.
* I found no visual defect beyond N8.

## Robustness (CHECK 5)

* **Crash.** I found no path to a crash. The int type is checked before `settings.int`, dates are validated, there is
  no arithmetic overflow, and the gzip loop ends on read=0/written=0 or an error.
* **Memory.** Every file is bounded (8 MiB read, 32 MiB inflated); there are at most 22 files, and the cache is capped
  at 32 entries.
* **Programs.** A child is killed after 5 s. A grandchild that keeps stdout open would block `reader.join()`; neither
  tool does that.
* **Remaining items.** M1, m1–m6.

## On GW4's decisions

1. A GW4 row is better than the sysmon text (m8).
2. Keep the rule, or ask GW2b to narrow the "no tool ran" assertion to the write methods. Either is fine.
3. Change the one-day form to "elke dag" / "every day" (m7).
4. Add the "Aanvullende opties" / "Additional options" key in round 3.
5. A GW7 `jobs.jsonl` line for the hand-over is right, but only after M1 is fixed. Otherwise a stale window already
   claims release upgrades.
6. The intermittent lib-suite hang needs an owner. My single full run did not hang.
