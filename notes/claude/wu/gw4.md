# GW4: Updategeschiedenis, Geavanceerde opties and D2 (2026-11-05)

Step GW4 of `notes/claude/windows-update-gnome.md` §10 (design §3.6, §4.3 `update.history`/`update.options`/
`update.setAutoOpen`, §4.5 `jobs.jsonl`, §4.11, §4.12, §8), in the worktree `.wt/gw4` (branch `wu/gw4`, from
f01710cc). Start state checked: `git status --short` empty, HEAD f01710cc. Nothing was committed. No bus, PackageKit,
apt/apt-config/snap, gsettings, dconf, win11-* binary or gnome-shell was run on the real system: every program ran
as a stub from a marked tool directory, every GSettings write went to a keyfile backend inside a fixture root, every
Chrome run inside `tests/update-ui/nonet.sh`'s layer, every heavy command through `tools/heavy.sh`. The only local
tools run outside a sandbox were `glib-compile-schemas` and `python3 -I` (fixture generation, writing only inside
`tests/update/fixtures/`). Scratch files are in `.rt/gw4/`.

## 1. Files

| File | What |
|---|---|
| `host/src/services/update/history.rs` | `update.history`: the apt history parser (plain, `.1`, `.1.gz`–`.4.gz` through GIO's in-process zlib converter, multi-member, a cut gzip gives its prefix), the release upgrader's own logs (`/var/log/dist-upgrade/history.log` and its `YYYYMMDD-HHMM/` backups), entries per package, groups, outcome, source, paging; 6 unit tests |
| `host/src/services/update/options.rs` | `update.options` (apt-config, snap, update-notifier's GSettings key) and `update.setAutoOpen` (D2, §4.12); 6 unit tests |
| `host/src/services/update/paths.rs` | only the `// GW4` block: `Paths.gsettings_schemas` / `gsettings_keyfile` (None in production; with a fixture root `<root>/usr/share/glib-2.0/schemas` and `<root>/home/.config/glib-2.0/settings/keyfile`) |
| `tests/update/fixtures/history/make.py` + `var/log/apt/*`, `var/log/dist-upgrade/**` | the history fixture root (generated, deterministic gzip); what each file holds is in make.py's docstring |
| `tests/update/fixtures/options/schemas/{com.ubuntu.update-notifier.gschema.xml, gschemas.compiled}` | the one key of update-notifier 3.207.2's schema (same id, path, type, default), compiled with `glib-compile-schemas` |
| `tests/update/fixtures/options/{apt-config-on,apt-config-off,apt-config-unset,snap-refresh-time,snap-refresh-time-held}.txt` | stub outputs |
| `vendor/ui/js/apps/settings/pages/update-history.js` | the page (§3.6 texts, makeT nl + en) |
| `vendor/ui/js/apps/settings/pages/update-advanced.js` | the page (§3.6 texts; the GW3 Software Updater row kept as it was) |
| `vendor/ui/js/mock/update.js` | only the GW4 section (history/options scenarios S01–S03h) and the three methods; the header line about the subpages |
| `vendor/ui/css/settings.css` | 14 lines at the end of the update block (`.st-updhist*`, one container query for `.st-updadv__row`) |
| `tests/i18n/surfaces.json` | 14 surfaces `settings-update-history.S01…S02b`, `settings-update-advanced.S03…S03h` + one `_comment` line |
| `tests/i18n/BASELINE-ADDITIONS.ui` | 24 `GW4` lines (the new nl keys) + 2 comment lines |
| `tests/i18n/baseline-nl.txt`, `baseline-en.txt` | re-recorded with `--record --accept settings.update-history,settings.update-advanced` |
| `vendor/ui-sha256.json` | regenerated (`ui-patches.json` unchanged in content) |
| `tests/update-ui/shots/{nl,en,en-narrow,nl-dark}/settings*.update-{history,advanced}*.png`, `shots/sheets/gw4-*.png` | screenshots (git-ignored) |

Not touched: `update.rs`, `tests.rs` and every other `update/*.rs` (GW2b), `host/i18n/update.tsv` (no new row, §5.3),
`api-methods.json`, `settings-defaults.json`, registry/main.js, every frozen file.

## 2. What was built

### 2.1 `update.history {before?, limit≤200}` → `{entries:[{group, name, version, date, ok, source}], more}`

* **Files** (all through `Paths`, so a test reads `WIN11_UPDATE_FIXTURE_ROOT`): `apt_log/history.log`, `.1`, `.1.gz`–`.4.gz`;
  `dist_upgrade_log/history.log` and up to 16 `YYYYMMDD-HHMM/history.log`. Ubuntu's release upgrader sets
  `Dir::Log=/var/log/dist-upgrade` (`DistUpgradeCache._initAptLog`, read on this machine), so a release upgrade
  never appears in `/var/log/apt/history.log`; `DistUpgradeMain.setup_logging` moves an earlier run's logs into a
  timestamped directory. Each such file is one release upgrade.
* **Entries per package.** `Upgrade:` → `updates`; `Install:` without `automatic` dependencies, `Reinstall:`,
  `Downgrade:` → `software`; removals are not listed. A release upgrade is one entry `releases` / `Ubuntu` /
  version from the upgrader's own package (`ubuntu-release-upgrader-core 1:26.04.14` → `26.04`). Multi-arch
  duplicates count once. Names and versions are validated (`model::valid_name`, Debian version characters).
* **ok** = no `Error:` line and an `End-Date:`. The newest apt run without `End-Date` is left out while apt's lock is
  held (`guard::apt_lock_held`: it is still running); otherwise (a crash, a cut log) it is failed. The current
  release log is left out while the upgrader runs (`guard::upgrader_running`). Both checks are asked only when needed.
* **source**: `ubuntu` (`Commandline` program `unattended-upgrade`), `us` (`packagekit …`, `Requested-By` our uid, the run
  inside one of our job windows: every `jobs.jsonl` line `{startedAt, finishedAt}` and an unfinished `job.json`
  until now; 2 s slack at the start, 5 s at the end), `terminal` (apt, apt-get, aptitude, dpkg), `app` (everything
  else: another PackageKit caller, aptdaemon = Software Updater, python-apt clients without a command line). A
  release upgrade has `us` inside one of our windows, else `null` (its log does not say how it was started).
* **Paging**: newest first; the entries before `before`, at most `limit` (default 200) and at most 90 days counted from
  the newest of them (so a call is never empty while older entries exist); `more` when older entries remain.
  `date` = `End-Date` (else `Start-Date`) in ms, plus `999 − index` inside one run, so the page's `before` = the
  last entry's date continues exactly, also when a limit cuts a run (test: a limit of 3 walks the whole fixture).
* Bytes that are not UTF-8 are replaced per line; a line cut inside a package list drops the unfinished item; every
  file is bounded (8 MiB read, 32 MiB decompressed); parsed files are cached by (size, mtime, inode, time zone).
  No program, no bus: the same in every mode (owner, mirror, reader, dry-run).

### 2.2 `update.options {}` → `{unattended, snaps:{last,next,hold}|null, autoOpen:{supported, on, days}}`

* `unattended`: `apt-config dump APT::Periodic::Unattended-Upgrade` (Tool::AptConfig). apt.systemd.daily's rule: unset,
  `"0"`, `"00"` → false; `"1"`, `"always"`, any other value → true; the last line of that exact key wins; `null` when
  apt-config fails.
* `snaps`: `snap refresh --time --abs-time` (Tool::Snap; `--abs-time` gives RFC 3339, checked in the installed snap
  binary's strings) → `last`, `next`, `hold` in ms or `null` (`n/a`; `next: … (but held)` reads the time); `null` when
  snap is missing or fails or prints no timer/schedule/last/next line.
* Both programs: only from the tool table (`Tools::command`: cleared environment + LANG + the table's PATH and dead
  test buses), stdin null, stdout ≤ 64 KiB, killed after 5 s. They run only in a `settings` host that is not dry-run
  and while Ubuntu's release upgrader does not run (§5.2); otherwise both rows are `null`.
* `autoOpen`: `com.ubuntu.update-notifier regular-auto-launch-interval` (int). Production: the default schema source
  (recursive lookup) and the default backend (dconf). Test mode: only `Paths.gsettings_schemas` (needs
  `gschemas.compiled`) and a `gio::keyfile_settings_backend_new(Paths.gsettings_keyfile, "/")`; test mode without a
  fixture root: no schema source and no backend at all, so a test can never reach the real dconf. No schema, no key,
  or a key that is not `i` → `{supported:false, on:false, days:null}` (the page hides the row). `on` = days < 36500;
  `days` ≥ 0.

### 2.3 `update.setAutoOpen {on}` (design §4.12)

Never reached in dry-run (`update.rs` answers `{dryRun:true}` first); a reader answers `not_here`; a mirror forwards
it to the owner (GW2). Off: `settings.update.autoOpenPrevious` = the user's own value (when the key had one) or
`"default"`, then the key = 36500. On: the remembered number (0…36499) is written back, otherwise the key is reset
(its default 7), then the setting goes back to `null`. A request equal to the current state changes nothing. In
production `gio::Settings::sync()` flushes dconf before the answer; in test mode it is skipped (the keyfile backend
writes at once, and `sync` would create the process's default backend). The answer is the `update.options` result.
Errors: no schema → `unsupported`, a key that is not writable → `failed`, both with `sysmon.startup.saveFailed`'s
text (§5.3); a failed `settings.set` is passed on as it is.

### 2.4 Pages (Windows 11 24H2 layout)

* **Updategeschiedenis**: one `SettingsExpander` per group in Windows' order, title with its count (`Updates (11)`,
  `Ubuntu-versies (1)`, `Software (3)`), no icon (as Windows' categories); Updates and Ubuntu-versies start open,
  Software closed (H's "the rest, collapsed"); an empty group is not shown. Each entry is a two-line row: `{name}
  {version}`, then `Geïnstalleerd op {date}` / `Kon niet worden geïnstalleerd op {date}` followed by the source phrase
  (`automatisch door Ubuntu`, `via {product}`, `via een app`, `in een terminal (apt)`; none for `null`). `{date}` is
  `format.js date(…,'short')` (25-10-2026 / 25/10/2026). "Meer laden" (standard button) under the groups while `more`;
  it asks with `before` = the oldest entry's date and keeps the groups' open state. Empty: `hEmpty`; a host without
  the method: common.js `unavailableHere`; any other failure: the host's message.
* **Geavanceerde opties**: first the toggle rows without icons, in Windows' order: `advMetered` + `advMeteredDesc`
  (`settings.update.metered`), `advNotify` (`settings.update.notifyRestart`), both through common.js `toggleCard`
  (= `settings.set`); then D2 `advAutoOpen` with its description (on: `advAutoOpenDays` {zero, one, other} + 
  `advAutoOpenDesc`; off: only `advAutoOpenDesc`, so the security-update exception is always there and the "gewone
  updates" sentence goes); then, as Windows' "Additional options", the rows with icons: `advSecurity` (shield;
  `advSecurityOn` / `advSecurityOff` / common.js `unavailable`), `advSnaps` (apps; `advSnapsDesc` with `{when}` and
  `{next}` rendered as the Update page renders `{when}`: `relTime (date time)`, "–" for a missing time; `unavailable`
  when null), and GW3's Software Updater row unchanged. The D2 toggle is disabled while its call runs; after an
  error, a dry run or a forwarded write it shows the real state again (`update.options`); a mirror waits ≤ 5 s for
  the owner's `update.reply` with its nonce and shows a refusal.
* CSS (update block only): the history list rows (11/12 px padding, divider, caption-size secondary date line,
  aligned with the header text at 16 px) and one container query: a narrow window wraps a card's control under its
  title with a 42 px indent meant for icon cards; the three toggle rows have no icon, so `.st-updadv__row` starts the
  switch under the title (the same fix GW3 made for its rows).

### 2.5 Mock states (`?updateScenario=`, surfaces `settings-update-history.<id>` / `settings-update-advanced.<id>`)

S01 three groups and every source (the registry page's default) · S01b more than 90 days ("Meer laden") · S01c after
"Meer laden" (the surface clicks it) · S01d failed entries (an update and a release upgrade) · S02 empty · S02b a
host without `update.history` · S03 all on, snaps times, auto-open every 7 days (default) · S03b the off states
(security off, auto-open off; seeded `notifyRestart` off, `metered` on) · S03c no update-notifier schema (row hidden) ·
S03d security updates and snaps unknown · S03e auto-open every 1 day, snapd never refreshed ("Laatst: –") · S03f S03
with the D2 toggle clicked (live → off text) · S03g 0 days ("meteen") · S03h a host without `update.options`. The
mock pages history exactly like history.rs (90 days, limit, `+999 − index`) and `setAutoOpen` remembers the previous
value like options.rs. The design's §9.3 ids S01/S02/S03 are kept; the letters are GW4's variants.

## 3. Verification (exact commands from W's root, results)

| Check | Command | Result |
|---|---|---|
| start state | `git status --short`; `git rev-parse HEAD` | empty; f01710cc |
| cargo check | `tools/heavy.sh --tag gw4-check -- cargo check --offline --locked -j2 --all-targets --message-format=short` | rc 0; lib 28, lib test 26, win11-launch 2, launch test 1 warnings (= baseline); **0** lines in `services/update` |
| update unit tests | `tools/heavy.sh --min-mb 2048 --tag gw4-test -- tests/rust-sandbox/run-tests.sh --lib -- services::update` | **57 passed**, 0 failed, 8 ignored (GW2b's skeletons); the 12 new ones: history 6, options 6; guard log 0 bytes |
| full sandboxed suite | `tools/heavy.sh --min-mb 2048 --tag gw4-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | see §3.1 |
| drift + ratchet | `python3 -B tools/i18n/drift_rust.py --warnings` (and `--paths host/src/services/update/{history,options,paths}.rs --warnings`) | **0 errors**, 2 warnings: `'gschemas.compiled'` (options.rs) and `'usr/share/glib-2.0/schemas'` (paths.rs), both "new literal with Dutch words, classed technical": GLib's fixed file and directory names ("schemas" looks like Dutch "schema's"), never shown to a user |
| catalogue | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` | **0 errors**, 72 warnings: 64 `[unused]` (the GW5–GW9 rows, as in GW2) + 8 of other stems / GW3 (glossary, space); none new |
| UI checker | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check.py` | **0 errors**, 124 warnings (glossary 52 = GW3's count, rust 72); the one new glossary note (`advNotify`: Windows' "a restart is required") carries a `glossary-ok` comment like GW3's `checking` |
| ratchet | `python3 -B tools/i18n/ratchet.py` | `ui: ok (89 allowed occurrences, 0 still to convert)` |
| dict diff | `PATH=… python3 -B tools/i18n/check_ui.py --dict-diff tests/i18n/dict-nl-3b898e6.tsv` | 2925 → 3164 rows; 9 changed, 20 removed, 259 added (GW3 235 + GW4 24); **0 not in BASELINE-CHANGES.ui, 0 stale** |
| i18n lists | `PATH=… python3 -B -m unittest tests/i18n/test_update_mock_rows.py` | 5 tests OK (1 skipped: the design-table variant, since update.tsv has rows) |
| node | `node --check --input-type=module < F` for update-history.js, update-advanced.js, mock/update.js | all ok |
| manifest | `python3 -B tools/ui-manifest.py`, then `--check`; `python3 -B scripts/verify-ui.py` | up to date (181 files; 65 changed, 20 added); OK, 181 files |
| CSS scope | `git diff -U0 vendor/ui/css` | one hunk `@@ -815,0 +816,14 @@`, after `.st-updnews__none` inside the update block |
| nl dump (before recording) | `tools/heavy.sh --tag gw4-dump -- tests/i18n/run.sh` (rc 1, the diff) and `… --out .rt/gw4/dump-nl.txt`, blocks against baseline-nl.txt | changed **only** `settings.update-history`, `settings.update-advanced` (GW3's placeholder pages, now filled); **14 new** blocks; 0 removed |
| en dump (before recording) | `tools/heavy.sh --tag gw4-dump -- tests/i18n/run.sh --lang en --out .rt/gw4/dump-en.txt` | the same 2 changed, 14 new, 0 removed |
| record | `tools/heavy.sh --tag gw4-dump -- tests/i18n/run.sh --record --accept settings.update-history,settings.update-advanced` and the same with `--lang en` | "changed block … (accepted)" ×2, 14 new blocks each; nl 24585 lines, en 24582 lines, 220 surfaces |
| compare | `tools/heavy.sh --tag gw4-dump -- tests/i18n/run.sh`, `… --lang en` (after recording, and again after the last page change) | "nl dump equals baseline-nl.txt (24585 lines)", "en dump equals baseline-en.txt (24582 lines)" |
| ge0a | `python3 -B -m unittest tests/i18n/test_ge0a.py` | OK (2 skipped: the heavy dump tests, run above) |
| HEAD → recorded | `blocks.blocks()` of `git show HEAD:tests/i18n/baseline-{nl,en}.txt` against the recorded files | nl and en: changed 2 (the two above), new 14, removed 0 |
| console | `--notes` of the dumps | only `taskmgr-dialog.restart-shell` (the sysmon offline warning, not GW4's) |
| shots | `tools/heavy.sh --tag gw4-shots -- tests/update-ui/shoot.sh <the 16 ids>` (twice; the advanced ids again after the narrow fix), `LANGS=en SIZE=800x1400 SUFFIX=-narrow … S01b,S01d,S03,S03b,S03e`, `THEME=dark LANGS=nl SUFFIX=-dark … S01d,S03` | 32 + 18 + 5 + 3 + 3 shots, 0 failed; sheets `shots/sheets/gw4-nl-adv.png`, `gw4-en-hist.png` |

### 3.1 Full suite

`tools/heavy.sh --min-mb 2048 --tag gw4-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast`, five runs:

* **Run 1: hang guard, exit 124**, before any `services::update` test had started (log kept in
  `.rt/gw4/test-full-run1-hang.log`): 79 tests passed, then 16 tests of `app_params`, `apps::catalog`, `apps::recent`
  and `files::{actions,places,properties}` hung. Thread states (`/proc/<pid>/task/*/wchan`): every test thread in
  `futex_do_wait`, 7 `apps::catalog` threads in `locks_lock_inode_wait` on the catalog's own `.apps-seen.lock`
  (held by the same process), one `apps::recent` thread blocked reading a socket. None of these files are GW4's,
  and none of GW4's code had run; it looks like the intermittent hang round 2 already noted (plan.md: "the one
  test hang", probably `files/read.rs`'s theme cache before `icon_catalog`). For the coordinator / round 3.
* **Run 2** (the done-when command again): rc 0. lib **378 passed** (366 + GW4's 12), **16 ignored** (8 + GW2b's 8
  bus skeletons), 0 failed; win11-launch 16 passed, 1 ignored; the other 11 binaries 0 tests. Oracles: 8
  `oracle_tests`, the `disks::tests` cases, `reference_contracts` all ok. `logs/guard-calls.log` **0 bytes**, tmp
  entries left 0, 3.1 s wall for the test run, 279 MB peak RSS.
* **Runs 3–5** (`tools/heavy.sh --min-mb 2048 --tag gw4-test -- bash -c 'for i in 1 2 3; do WIN11_TEST_TIMEOUT=300
  tests/rust-sandbox/run-tests.sh --no-fail-fast; …; done'`): 3 × rc 0, 378 passed / 16 ignored, guard log 0, tmp 0.

## 4. Screenshot review (against Windows 11 24H2 Settings › Windows Update, from memory; no reference set here)

* **Update history**: breadcrumb "Update en herstel › Updategeschiedenis", the category expanders with "(n)" as in
  24H2 ("Kwaliteitsupdates (2)"), no icons, chevron right; inside, each update is a two-line row (body text, then the
  caption-size secondary "Geïnstalleerd op …") with dividers, as Windows. Windows' "Meer informatie" link per entry and
  the "Gerelateerde instellingen" (Updates verwijderen, Herstel) are not there (no URL per apt package; no uninstall),
  and "Meer laden" is ours (Windows loads everything). Failed entries read as Windows' "Kon niet worden geïnstalleerd
  op …" (no error code). Dark mode correct.
* **Advanced options**: the three toggle cards without icons with "Aan/Uit" left of the switch, then the icon rows,
  then the Software Updater row with its standard "Openen" button. Windows puts an "Aanvullende opties" header above
  its icon rows; the design has no key for it, so here a group gap separates them (decision 6.4).
* **English widths**: at 1280 px every title is one line; D2's description takes two lines in nl and en. At 800 px the
  long D2 title wraps to two lines, the descriptions to 2–3 lines, controls wrap under the title (left-aligned after
  the fix), the snaps line wraps at "(25/10/2026 07:35)"; no clipping or overflow. The breadcrumb's last item is cut
  with "…" at 800 px ("Update hi…", "Advanced…"), as GW3's subpages already were (the shared breadcrumb, not GW4's).

## 5. Deviations from the design and contracts

1. **Release upgrades come from `/var/log/dist-upgrade/`**, not from `/var/log/apt/history.log*` (the design says
   "from history.log and .1.gz–.4.gz"): the upgrader redirects apt's logs there (read in `DistUpgradeCache.py:226`).
   One entry per upgrade, version from the upgrader's own package. A release upgrade's `source` is `null` unless one
   of our job windows covers it (GW7 can add a `jobs.jsonl` line for its hand-over; nothing else is needed).
2. **`update.options` runs no program while Ubuntu's release upgrader runs** (both rows `null`). Reason in the code:
   snapd restarts during an upgrade. Practical reason: GW2's `every_write_is_refused_while_the_release_upgrader_runs`
   (tests.rs, not mine) calls every method of an owner and asserts that no tool ran at all, reads included; without
   this rule the suite is red. If GW2b narrows that assertion to writes, the rule can stay or go.
3. **No new `update.tsv` row.** A `#@ new` row needs a `key:` line in a `BASELINE-CHANGES.rust*` file, and GW4 owns
   none (drift_rust would fail). The two setAutoOpen errors use the existing `sysmon.startup.saveFailed` ("De
   instelling kon niet worden opgeslagen." / "The setting couldn’t be saved."). Alternative (decision 6.1): a row
   `update.options.err.autoOpen` plus `tests/i18n/BASELINE-CHANGES.rust-gw4`.
4. **Response fields**: `snaps` has an extra `hold`; `autoOpen.days` is `null` when unsupported; entries have
   `source:null` for an unknown release-upgrade source. All additive to §4.3.
5. **Dates are ms with an ordering part** (`End-Date + 999 − index`), so `before` pages exactly; the displayed date is
   unaffected. Two apt runs ending in the same second could overlap (not seen in practice).
6. **D2 texts**: §3.6's `advAutoOpenDesc` (two sentences, the first dropped when off) is split into `advAutoOpenDays`
   (first sentence, plural) and `advAutoOpenDesc` (the rest). `advAutoOpenDays` has a **zero** form of mine
   ("Ubuntu opent de Software-updater meteen als er gewone updates zijn." / "… right away when regular updates are
   available."), because update-notifier's 0 means "at once" and "om de 0 dagen" would be wrong. The `one` form is the
   design's literal "om de {n} dag" / "every {n} day" (decision 6.3).
7. **Who installed it** is shown after the date line in one sentence ("Geïnstalleerd op 25-10-2026 automatisch door
   Ubuntu"), the design's lower-case phrases joined with a space. Removals and automatic dependencies are not listed.
8. **Dry-run**: §4.3 says `update.options` is "same" in dry-run, §4.11 says no subprocess in dry-run: the second wins,
   so a dry Settings host shows the two read-only rows as "Niet beschikbaar" (the auto-open row still reads GSettings).
9. **gsettings in test mode is a fixture path** (`<root>/home/.config/glib-2.0/settings/keyfile`), not `$HOME`, so it is
   re-rooted with the other system files and never depends on the caller's HOME or GSETTINGS_BACKEND.
10. GW3's stub of `update-advanced.js` kept its four keys and texts byte for byte; `update-history.js` keeps `hEmpty`.

## 6. Decisions for the coordinator

1. Accept the reuse of `sysmon.startup.saveFailed` for setAutoOpen's two errors, or give GW4 (or round 3) a
   `BASELINE-CHANGES.rust-gw4` so an own row `update.options.err.autoOpen` can be added.
2. Deviation 2: accept the "no programs while the upgrader runs" rule, or ask GW2b to restrict the "no tool ran"
   assertion in `every_write_is_refused_while_the_release_upgrader_runs` to the write methods.
3. Deviation 6: keep "om de 1 dag" (design) or use "elke dag" / "every day" as the `one` form (one row each).
4. Windows' "Aanvullende opties" / "Additional options" header above the icon rows of Geavanceerde opties needs one
   new key (not in §3.6); today a group gap stands in for it.
5. GW8 (§9.3) can use S01/S02/S03 as written; the lettered ids are GW4's text variants.
6. GW7: to make a release upgrade read "via Win11 Shell", append a `jobs.jsonl` line `{jobId, role:'release',
   startedAt, finishedAt}` for the hand-over window; history.rs matches any role.
7. MOCK-CHANGES (GW3 D4, round 3): `mock/update.js` now also answers `update.history` / `update.options` /
   `update.setAutoOpen` per S-scenario.
8. The intermittent lib-suite hang of §3.1 run 1 (apps::catalog / apps::recent / files::* threads, 1 of 5 runs here)
   needs an owner in round 3; GW4's tests were not involved.
9. Fixtures: `python3 -I -B tests/update/fixtures/history/make.py` rewrites the history fixture root;
   `glib-compile-schemas tests/update/fixtures/options/schemas` the compiled schema. Both outputs are committed so
   `cargo test` needs neither tool.

## 7. Left / not done

* No GNOME-harness (WebKitGTK) run of the real host with the fixture root; the pages were checked in mock mode only
  (headless Chrome). The real `update.history` against this machine's own logs and the real dconf write are R8 /
  GW8 matters (an agent must not run them).
* Bus tests are GW2b's; nothing of GW4 needs the fake PackageKit.

## 8. Fix round (after `notes/claude/wu/gw4-review.md`, 2026-11-05)

Start state checked: `git status --short` and `git diff --stat` equal to the review's view (12 modified, 3 untracked:
`gw4.md`, `gw4-review.md`, `tests/update/fixtures/`; 3175+/54−). Same rules as above: nothing committed, no real bus,
apt, snap, gsettings or dconf; every heavy command through `tools/heavy.sh` (tags `gw4-fix-*`); scratch output in
`.rt/gw4/fix/` (the files from before the round are kept there as `*.before`).

### 8.1 Findings

| Id | Status | What changed |
|---|---|---|
| **M1** stale `job.json` | **fixed** | `history::job_windows(store, paths, now)`: an unfinished `job.json` lasts until now only when `bootId` = `store::boot_id` **and** `store::owner_alive(owner)` (update.rs's own filter); otherwise its window ends at the file's last write (`store.modified_ms(JOB)`, never later than now). Test `a_stale_job_file_ends_its_window_at_its_last_write_…`: this boot + live owner → now; other boot, dead owner (same pid, other start), no owner → mtime; GNOME Software's run a day later is `app`, the run inside the window `us` |
| **m1** paging | **fixed** | Every entry has a unique `cursor` = `second << 20 \| slot` (blocks sorted chronologically, the slots of one second in log order; < 2^53 so JavaScript keeps it exact; dates limited to 2000–2200 for that). `before` ≥ 2^48 is a cursor (`cursor < before`), below it a time in ms (the design's form, `date < before`). `date` is now the exact `End-Date` (no `+999 − index`). Test: a 1200-package run plus two runs ending in the same second, walked at limit 200 and limit 1: all 1204 exactly once, in order; the fixture walked at limit 3 |
| **m2** partial upgrade | **fixed** | `Run.release`: an `Upgrade:` item of the upgrader's own packages whose old and new versions name different releases (`1:25.10.9 → 1:26.04.14`). A `dist-upgrade` log is one `releases` entry only then; any other (Software Updater's partial upgrade) is listed like apt's runs (per package, `updates`/`software`, source `app` as it has no command line; the "newest run without End-Date while apt's lock is held" rule covers it while it runs). The fixture now has the realistic layout: the current `dist-upgrade/history.log` is a partial upgrade of 10 Oct, the 20 Aug release upgrade moved to `20261010-1830/`, the failed June attempt stays in `20260820-1012/`. base-files is not used (its version names no release) |
| **m3** 90-day horizon | **fixed** | `Context.horizon` = 90 days before `jobs.jsonl`'s mtime (each append drops what finished 90 days earlier), at the latest 90 days before now. A PackageKit run of our uid without a window that ended before it gets `source: null` (no phrase on the page), not `app`; a window that is still there still gives `us` |
| **m4** big plain log | **fixed** | `read_tail`: a plain file over 8 MiB is read from `len − 8 MiB` and starts at the first `\nStart-Date:` (one extra byte decides whether the tail starts at a line start). `gunzip_tail` keeps the last 32 MiB of what it inflates (at most 256 MiB inflated), from its first whole run when the front was dropped. Tests with small bounds against the fixture files |
| **m5** blank page | **fixed** | `update-advanced.js` builds every card at once; the security, Snap Store and D2 rows say common.js `loading` ("Even geduld…" / "Just a moment…") until `update.options` answers, D2's switch is hidden and disabled until its state is known (no false "Uit"), and the row is removed when the schema is missing. `options.rs` runs `apt-config` and `snap` at once (`thread::scope`): at most 5 s instead of 10 s. New mock state **S03i** (update.options never answers) and surface `settings-update-advanced.S03i` |
| **m6** lost mirror write | **fixed** | Page: no `update.reply` within 5 s → it reads `update.options` and shows common.js's `showError` toast ("Dat lukte niet") unless the requested state arrived anyway. Owner: `update.setAutoOpen` starts no program any more: it answers in the `update.options` shape with a fresh `autoOpen` and this host's last remembered `unattended`/`snaps` rows (per cache directory, `null` before the first `update.options`), so the forwarded reply follows the write at once. The page takes only `autoOpen` from that answer. Tested: `options_come_…` (same rows, no run), `auto_open_round_trips_…` (0 runs, null rows) |
| **m7** "om de 1 dag" | **fixed** | `advAutoOpenDays.one`: "Ubuntu opent de Software-updater elke dag als er gewone updates zijn." / "Ubuntu opens Software Updater every day when regular updates are available."; the `BASELINE-ADDITIONS.ui` line follows. This replaces §3.6's literal one form (decision 8.4.2) |
| **m8** sysmon text | **not fixed** | A new `update.options.err.autoOpen` row needs a `key:` line in a `tests/i18n/BASELINE-CHANGES.rust*` file (`drift_rust.py` rule 3 / `#@ new`), and GW4 owns none. The errors still use `sysmon.startup.saveFailed` (decision 8.4.1) |
| **m9** held snaps | **partly fixed** | `snaps.hold` (unused) is replaced by `held` (bool: snapd printed a `hold:` line, a time or `forever`, both seen in the installed snapd's strings), and while held `next` is when the hold ends (`null` for forever) instead of snapd's "… (but held)" time, so the shown "volgende" is right. A text for the held state ("updated automatically" is not true then) is not in §3.6, and new texts must come from the design: round 3 (decision 8.4.3) |
| N1 GVfs connect | correction | The keyfile backend loads GVfs, which connects once to `$DBUS_SESSION_BUS_ADDRESS` (the sandbox's private bus). §2.2's "no backend at all" is about test mode *without* a fixture root and stays true; with a fixture root the only bus contact is that private session bus. `GIO_USE_VFS=local` for the test environments is the coordinator's (harness, `run.sh`, `run-tests.sh`) |
| N2, N3 | unchanged | GW8/GX1: every Settings run that can open the subpages must set `WIN11_UPDATE_TOOL_DIR` and `WIN11_UPDATE_FIXTURE_ROOT` (else production paths: `/usr/bin/snap` against the visible `/run/snapd.socket`, the real `/var/log/apt`) |
| N4 schema checks | **fixed** | `SettingsSchemaSource::from_directory(dir, None, false)` (GLib checks the compiled file), and a schema without a path (relocatable) gives `None` before `Settings::new_full` |
| N5 | – | nothing to do; `paths.rs` unchanged in this round (the same two `// GW4` hunks) |
| N6/N8 24H2 keys | unchanged | design keys for round 3 (decision 8.4.5) |
| N7 focus | **fixed** (focus) | "Meer laden" keeps keyboard focus on the new button while there is more (captured before the button is disabled). The group count counting the loaded entries is kept: it is the count of what the list shows |

### 8.2 Files changed in this round

`host/src/services/update/history.rs` (8 tests now, 2 new), `host/src/services/update/options.rs` (6 tests, extended),
`tests/update/fixtures/history/make.py` and its regenerated `var/log/dist-upgrade/{history.log, 20261010-1830/history.log}`
(the apt files are byte-identical), `vendor/ui/js/apps/settings/pages/update-advanced.js`, `update-history.js`,
`vendor/ui/js/mock/update.js` (GW4 section: exact dates and cursors like history.rs, `held`, S03i),
`tests/i18n/surfaces.json` (+1 surface, comment), `tests/i18n/BASELINE-ADDITIONS.ui` (the one-form line and its
comment), `tests/i18n/baseline-{nl,en}.txt` (re-recorded), `vendor/ui-sha256.json` (regenerated). Not touched:
`paths.rs` (as before), `update.tsv`, `settings.css`, every file of GW2b/GX1, every frozen file.

API changes (all additive or within the types `update.rs` checks): entries carry `cursor`; `before` takes a cursor or
ms; `date` is the exact `End-Date`; `snaps` is `{last, next, held}`; `update.setAutoOpen`'s `unattended`/`snaps` are the
host's remembered rows.

### 8.3 Verification (from W's root)

| Check | Command | Result |
|---|---|---|
| cargo check | `tools/heavy.sh --tag gw4-fix-check -- cargo check --offline --locked -j2 --all-targets --message-format=short` | rc 0; warnings lib 28, lib test 26, launch 2, launch test 1 (= baseline); **0** in `services/update` |
| update tests | `tools/heavy.sh --min-mb 2048 --tag gw4-fix-test -- tests/rust-sandbox/run-tests.sh --lib -- services::update` | **59 passed**, 0 failed, 8 ignored; GW4's 14 (history 8, options 6) all ok. The first run failed 1: GW2's `no_absolute_exec_path_outside_the_tool_table` caught a `"/usr/bin/unattended-upgrade"` literal in my new test data; the literal is now `"unattended-upgrade"` |
| full suite | `tools/heavy.sh --min-mb 2048 --tag gw4-fix-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | rc 0; lib **380 passed**, 0 failed, 16 ignored; win11-launch 16 passed, 1 ignored; the 11 other binaries 0 tests; 9 oracle tests ok; `logs/guard-calls.log` **0 bytes**; tmp left 0; 14.6 s wall. One run, no hang |
| fixtures | `python3 -I -B tests/update/fixtures/history/make.py`; the same script copied to `.rt/gw4/fix/mk/` and `diff -r` | regenerated; the copy reproduces `var/` byte for byte |
| drift | `python3 -B tools/i18n/drift_rust.py --warnings` | **0 errors**, 2 warnings (the GLib names, as before) |
| check_rust | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` | **0 errors**, 72 warnings (none new) |
| check.py | `PATH=… python3 -B tools/i18n/check.py` | **0 errors**, 124 warnings (glossary 52, rust 72) |
| ratchet | `python3 -B tools/i18n/ratchet.py` | `ui: ok (89 allowed occurrences, 0 still to convert)` |
| dict diff | `PATH=… python3 -B tools/i18n/check_ui.py --dict-diff tests/i18n/dict-nl-3b898e6.tsv` | 2925 → 3164 rows; 9 changed, 20 removed, 259 added; **0 not in BASELINE-CHANGES.ui, 0 stale** |
| i18n tests | `python3 -B -m unittest tests/i18n/test_update_mock_rows.py`; `… tests/i18n/test_ge0a.py` | 5 OK (1 skipped); 21 OK (2 skipped). (Before S03i was recorded, test_ge0a failed 2: the new surface had no baseline block yet; green after recording) |
| node | `node --check --input-type=module < F` for the three modules | 3 × ok |
| manifest | `python3 -B tools/ui-manifest.py`, `--check`; `python3 -B scripts/verify-ui.py` | up to date (181 files; 65 changed, 20 added); OK, 181 files; `ui-patches.json` unchanged in content |
| CSS scope | `git diff -U0 vendor/ui/css` | still one hunk `@@ -815,0 +816,14 @@` (no CSS change this round) |
| dumps before recording | `tools/heavy.sh --tag gw4-fix-dump -- tests/i18n/run.sh --out .rt/gw4/fix/dump-nl.txt` and `--lang en --out …dump-en.txt`, then `python3 -I tests/i18n/blocks.py tests/i18n/baseline-{nl,en}.txt <dump>` | nl and en: exactly **1 changed block**, `settings-update-advanced.S03e` (the one form), 0 new, 0 removed |
| record | `tools/heavy.sh --tag gw4-fix-dump -- tests/i18n/run.sh --record --accept settings-update-advanced.S03e` and `--lang en …`; after adding S03i the same with `--accept settings-update-advanced.S03i` | "changed block … (accepted)"; then "new block settings-update-advanced.S03i"; nl 24638 lines, en 24635 lines |
| compare | `tools/heavy.sh --tag gw4-fix-dump -- tests/i18n/run.sh`, `… --lang en` (after the last page change) | "nl dump equals baseline-nl.txt (24638 lines)", "en dump equals baseline-en.txt (24635 lines)" |
| HEAD → recorded | `blocks.blocks()` of `git show HEAD:tests/i18n/baseline-{nl,en}.txt` against the recorded files | nl and en: changed only `settings.update-history`, `settings.update-advanced` (GW3's placeholders); **15 new**; 0 removed. Dutch that existed before is unchanged |
| console | dumps' `--notes` | only `taskmgr-dialog.restart-shell` (not GW4) |
| shots | `tools/heavy.sh --tag gw4-fix-shots -- tests/update-ui/shoot.sh <17 ids>` (the two registry pages, S01–S02b, S03–S03i); `LANGS=en SIZE=800x1400 SUFFIX=-narrow … S01b,S01d,S03,S03b,S03e,S03i`; `THEME=dark LANGS=nl SUFFIX=-dark … S01d,S03,S03i` | 34 + 6 + 3 shots, 0 failed |

Screenshot review (against Windows 11 24H2, as in §4): S03i shows the page at once with the three program rows on
"Even geduld…" and no switch on D2 (light, dark, en 800 px: the title wraps, no overflow); S03e reads "elke dag" /
"every day" on two lines at 1280 px and three at 800 px; S03c drops the D2 row; S03f shows the off text after the
click; S03h shows "Niet beschikbaar"/"Not available" and no D2 row; the history states are unchanged (S01c lists
libexpat1 and libtiff6 after "Meer laden", Updates (13)). The 24H2 gaps of N6/N8 remain.

### 8.4 Decisions for the coordinator (updated)

1. m8: grant `tests/i18n/BASELINE-CHANGES.rust-gw4` (then `update.options.err.autoOpen` through `extract_rs.py add`), or
   accept the reuse of `sysmon.startup.saveFailed`.
2. m7: confirm the one form "elke dag" / "every day" as the amendment of §3.6's `advAutoOpenDesc` plural.
3. m9: a design text for held Snap refreshes (`snaps.held` is in the API; the row keeps §3.6's text until then).
4. §5 deviation 2 (no programs while the release upgrader runs) is unchanged; GW2b may narrow its test instead.
5. Round 3: the "Aanvullende opties" header, a description for the restart-notification row and "Gerelateerde
   instellingen" on Updategeschiedenis (N6/N8).
6. N1: `GIO_USE_VFS=local` in `tests/rust-sandbox/run-tests.sh`, `tests/update/run.sh` and the harness.
7. N2/N3: GW8/GX1 set `WIN11_UPDATE_TOOL_DIR` and `WIN11_UPDATE_FIXTURE_ROOT` (with the options schema copied into it)
   for every Settings run that can open these subpages.
8. GW7's `jobs.jsonl` line for the release hand-over is now safe (M1 fixed: a stale `job.json` no longer claims later
   runs). If round 3 keeps `jobs.jsonl` as long as the rotated logs (~150 days), m3's `null` sources become `us` again.
9. The intermittent lib-suite hang (§3.1 run 1) still needs an owner; it did not recur in this round's run.

### 8.5 Left

* No GNOME-harness (WebKitGTK) run of the real host; the pages were checked in mock mode (headless Chrome).
* m8 and the held-state text of m9 (decisions 1 and 3).
