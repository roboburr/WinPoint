# GW6: the compatibility check and the feature-update card (2026-11-06)

Step GW6 of `notes/claude/windows-update-gnome.md` §10 (design §3.5, §3.7 `update.compat.*`, §4.3, §4.5, §4.11, §6.2,
§9, §11 R5/R6/R10, §12 C4/C13/C14/C16/C17; plan.md D-G12, D-G26, D-G27, D-G51), in the worktree `.wt/gw6` (branch
`r4/gw6`, from 4e0323eb). Start state checked: `git status --short` empty, HEAD 4e0323eb. Nothing committed. Scratch
files in `.rt/gw6/`, test runs in `tests/update/.run/compat-*` (git-ignored), screenshots in
`tests/update-ui/shots/{nl,en}-gw6{,-narrow,-dark}/` (excluded from git, D-G22).

Live-session rules kept: no PackageKit, NetworkManager, polkit or system/session bus call, no check-new-release,
do-release-upgrade, pkexec, gsettings or dconf, no win11-* binary or gnome-shell, no network. apt-get, apt-cache and
apt-config ran only under D-G27 (below: inside bwrap with `--unshare-net`, a private `/run` and `/var`, the fixture's
`/etc/apt`, a private apt tree, a `file:` archive signed with a throwaway key, as Robin's uid). **One slip, for the
record:** at the very start, while checking which tools exist, I ran `apt-get --version` once outside any layer (it
prints apt's version and reads apt's configuration; no cache, list, lock or dpkg state, no network). Nothing else of
apt ever ran outside the layer. Every heavy command went through `tools/heavy.sh`.

## 1. Files

| File | What |
|---|---|
| `host/src/services/update/elf.rs` | the `DT_NEEDED` parser (64-bit LE ELF, `PT_DYNAMIC`, `DT_STRTAB` mapped through its `PT_LOAD`; reads header, program headers, dynamic segment and strings only), the skip rule (libc, libgcc_s, ld-linux); 4 unit tests |
| `host/src/services/update/compat.rs` | the cheap verdict, the full check (sonames → packages, the private apt tree, apt-config/update/policy/simulation), the parsers, the verdict, the cache, `update.release.compat`, `compat::cached`, `compat::fill` (the snapshot's card fields); 13 unit tests + 1 ignored D-G27 layer test |
| `host/data/releases.json` | new, exactly design §6.2's file (resolute tested 50; stonking untested 51, `allowAnyway:false`), `include_str!` |
| `host/src/services/update/paths.rs` | one `// GW6` part in the struct and one in `rooted()` (as GW4/GW5): `user_data`, `bin_dir`, `releases` (fixture root only), `locale_dirs`; the keyring, dpkg status and extended_states were already re-rooted by GW2 |
| `host/src/services/update/release.rs` | `// GW6` blocks only: in `status()` the bound object (`let mut view =`), `compat::fill`, and `restore_hold` when the verdict allows the upgrade or another release is offered; in GW5's test `ubuntu_hold_on_needs_an_offer_on_hold_and_a_schema` a GW6 block (the cheap hold now allows the postponement; the status restores it for another codename) |
| `vendor/ui/js/apps/settings/pages/update-release.js` | the card (every state), the start dialog, 42 new makeT keys (44 rows with the plural forms); the Prompt row untouched |
| `vendor/ui/js/mock/update.js` | scenarios C01–C16 (25 ids with letters), `offer()`, the release object's GW6/GW7 fields, `renderReason`, live `update.release.compat`/`ubuntuHold`/`start`, 13 host rows (all existing `update.tsv` rows: `update.compat.*`, `update.part.{webkit,jsc,archive,ffmpeg}`, `update.feature.ffmpeg`) |
| `vendor/ui/css/settings.css` | one `/* GW6 */` block at the end of the update block (`.st-updrel*`) |
| `vendor/ui-sha256.json` | regenerated (`tools/ui-manifest.py`; `ui-patches.json` unchanged) |
| `tests/i18n/surfaces.json` | 25 `settings-update-card.*` surfaces + one `_comment` line (appended) |
| `tests/i18n/BASELINE-ADDITIONS.ui` | 44 GW6 lines + 2 comment lines (appended) |
| `tests/i18n/baseline-{nl,en}.txt` | re-recorded with `--record`: only the 25 new card blocks (and the header's surface count / END line); no existing block changed |
| `tests/update/mkarchive.py` | new: throwaway key, signed `file:` archives (ok, warn, hold, blocked, ubuntuHold), fixture dpkg status + extended_states |
| `tests/update/test_compat.sh` | new: the D-G27 test (readelf, stage 1 key/archives, stage 2 real apt under strace, checks, `--record`) |
| `tests/update/fixtures/compat/**` | new: `readelf-needed.txt`, `dpkg-query-S.txt`, `root/` (os-release, `ubuntu.sources` with two Ubuntu stanzas of their own components, two third-party sources, the extension's metadata.json), `system-apt/` (the fixture `/etc/apt` with Ubuntu's hook shapes and a proxy), `recorded/` (the real apt outputs of the D-G27 run per case, replayed by the unit tests) |
| `notes/claude/api-docs-gw6.md` | the doc strings for `api-methods.json` (WU-FIX) |

Not touched: `update.rs`, `tests.rs`, `bus_tests.rs`, `upgrade.rs`, every other `update/*.rs`, `host/i18n/update.tsv`
(no new row was needed, so no `BASELINE-CHANGES.rust-gw6`), `api-methods.json`, `update.js`, frozen files.

## 2. What was built

### 2.1 elf.rs
`needed(path)` returns the NEEDED sonames in the dynamic section's order; a static file gives none, anything malformed
(not ELF, 32-bit, big-endian, truncated, > 256 program headers, > 1 MiB tables, a string outside its table) an error,
never a panic. `skipped()` drops libc, libgcc_s and ld-linux. Our 12 binaries need `libgdk-3.so.0 libz.so.1
libwebkit2gtk-4.1.so.0 libgtk-3.so.0 libgdk_pixbuf-2.0.so.0 libcairo.so.2 libsoup-3.0.so.0 libgio-2.0.so.0
libjavascriptcoregtk-4.1.so.0 libgobject-2.0.so.0 libglib-2.0.so.0` (the GTK hosts), less for the workers and
win11-launch (`fixtures/compat/readelf-needed.txt`).

### 2.2 compat.rs (design §6.2)
* **Cheap verdict first.** `releases.json` (embedded; with a fixture root `<root>/releases.json` if present) and the
  installed `metadata.json` under `$XDG_DATA_HOME/gnome-shell/extensions/<uuid>/`. Not `tested`, or the entry's
  `gnomeShell` not matched by any `shell-version` entry (GNOME's rule: `"50"` or `"50.x"`), or no extension → `hold`;
  `update.release.compat` then answers `{started:false, skipped:'hold'}` and starts nothing. Today: stonking holds.
* **Full check** (`run_full`, only for a tested codename, on the `update-compat` thread under `compat.lock`):
  1. sonames of every `win11-*` next to the running program (fixture: `<root>/bin`) + `libarchive.so.13` →
     `dpkg-query -S <soname paths>` (tool table) → the blocking packages; `libarchive13t64` when dpkg names no owner.
  2. sources: each enabled `deb` stanza of `ubuntu.sources` that names the current codename's suites, with the
     codename's `<c>`, `<c>-updates`, `<c>-security` (backports/proposed dropped), its **own** URIs and components,
     `Signed-By` forced to `Paths.archive_keyring`; http(s) only (`file:` only in test mode), nothing that could break
     a deb822 field.
  3. `apt-config dump` (the system's) → `Acquire::http::Proxy`, copied when it is a plain value.
  4. the tree `<cache>/update/release-check/<codename>/` (0700): `etc/apt/apt.conf` (every `Dir::Etc*`, `Dir::State`,
     `Dir::Cache`, `Dir::Log` inside the tree, `Dir::State::status`/`extended_states` the system's (re-rooted in test
     mode), `Debug::NoLocking`, `APT::Architectures {amd64}`, no translations, no Contents/DEP-11/CNF), an empty
     `apt.conf.d`, `sources.list.d/release.sources`.
  5. `apt-config dump` with `APT_CONFIG` must show no hook key (`…invoke…`, `…::hooks…`, `pre-install-pkgs`) and every
     `Dir::Etc*`, lists and cache directory inside the tree, else no apt run at all (`unknown`, apt reason).
  6. `apt-get -q update` (15 min), `apt-cache policy gnome-shell <blocking> <degraded>`, `apt-get -s -q dist-upgrade`
     (5 min each), nice 10 and idle I/O in the child, `LANG=C LC_ALL=C`, the proxy variables in production only.
  7. the tree is removed (a guard's `Drop`), whatever happened.
* **Verdict** (`evaluate`): a blocking package (or gnome-shell) removed by the simulation or without any archive
  version, unless a renamed package of the same library is installed instead (t64-style stem), → `blocked` with one
  `update.compat.removed`/`.missing` reason each; else the archive's gnome-shell major (the simulation's new version,
  else the archive candidate) not in `shell-version` → `hold` (`gnomeMajor` named; a different `releases.json` major
  is a note in the cache, never a reason); else a degraded group gone (only groups with an installed package; the
  terminal group is "any of ptyxis/gnome-terminal/gnome-console", xdg-terminal-exec counts as part "terminal") or
  third-party sources (files in `sources.list.d` but `ubuntu.sources` with an active non-`*.ubuntu.com` entry; the
  first one's host named) → `warn`; else `ok`. `ubuntuHold` is applied when the snapshot is read (`decide`):
  `UpgradeBroken` turns ok/warn/unknown into ubuntuHold, blocked and hold stay.
* **Cache** `compat-<codename>.json`: `{version, codename, state, reasons, gnomeMajor, checkedAt, archiveDate,
  inputs:{extensionVersion, shellVersion, entry, version}, notes, blocking}`; valid 7 days (a failed check 1 h) while
  the inputs are equal; reasons stored as keys + params, rendered per call.
* **`update.release.compat {refresh?}`**: `{started:true}`, or `{started:false, skipped}` = `notOffered`, `hold`,
  `recent` (a valid verdict; `refresh` re-runs only a failed one), `offline` (NM not `online`), `metered` (without
  `update.metered`), `running`. `update.changed` when it starts and when it ends. After an ok/warn verdict:
  `release::restore_hold`.
* **`compat::fill`** (called by `release::status` for every host; files only): `state`, `verdict`, `reasons`,
  `gnomeMajor`, `allowAnyway`, `compat {running, needed, checkedAt}`, `upgraderNl`. No verdict yet: `checking` with
  `needed:true` (the page asks for the check), or `unknown` with the offline/metered reason while it may not run, or
  `unknown` without a reason in a dry-run host.
* **`compat::cached(shared, codename)`**: the verdict known without running anything, the cheap hold included (GW5's
  `ubuntuHold {on:true}` needs `hold`/`blocked`, which is today's case).

### 2.3 The card (design §3.5)
Windows 11 24H2's feature-update card: a 48 px logo (`distributor-logo`, then Ubuntu's pixmap, then the update glyph
in a disc; the mock has no themed icons, so it shows the glyph), the title in BodyStrong, the text and links under it,
the button at the right (under the text, aligned with it, below 476 px). States: checking (ProgressRing +
relChecking), ok, warn (+ the attached Expander "Let op voordat je upgradet" with the reasons), hold (relHoldGnome
when the archive named the major, else relHoldBody; the muted relUbuntuMayOffer line and the click-only C14 link
`update.release.ubuntuHold {on:true}`, then relUbuntuWaiting + "Ongedaan maken"; `allowAnyway` adds "Toch upgraden…"),
ubuntuHold, blocked (one host line per reason + the same Ubuntu line/link), updatesFirst (`update.install`, then the
start dialog when the offer is ok/warn again; held names as a muted line), rebootFirst (the hero's restart flow),
running (only next to our own job: the hero says the same during releaseRunning), waiting, startFailed (the host
reason; Retry back to the offer on this page), ended ("Logboek openen" = `files.open` of main.log; Sluiten hides it
until the next check), unknown (relCheckFailed + the host reason; Retry = `update.release.compat {refresh:true}`).
The start dialog: relStartQ, relStartBody, then the battery (`system.battery.get`: present and not on AC), metered
(`network.metered`) and, in Dutch while `upgraderNl` is false, English-first InfoBars; "Upgrade starten" →
`update.release.start {}`. "Toch upgraden…" opens the same dialog with the hold text as a warning InfoBar on top and
"Toch upgraden" as its button. A refusal of any card write is a closable error InfoBar under the card; a mirror
window waits for `update.reply`; a dry run gets the usual toast (the automatic compat call is quiet).

## 3. Contract with GW7

GW7 (running in parallel) owns the hand-over states. The card renders exactly these fields of `update.status`'s
`release` object; GW7 writes the ones marked GW7, GW6 the ones marked GW6, GW5's stay as they are.

| Field | Owner | Meaning for the card |
|---|---|---|
| `state` | GW6, then GW7 | GW6: `none\|checking\|ok\|warn\|hold\|ubuntuHold\|blocked\|unknown`. GW7 may replace it with `updatesFirst\|rebootFirst\|running\|waiting\|startFailed\|ended`. Recommended precedence: running/waiting/startFailed/ended (any verdict) > updatesFirst > rebootFirst > GW6's state, and updatesFirst/rebootFirst only when `verdict` is `ok`/`warn` (otherwise the hold/blocked card stays). `compose` keeps forcing `running` while the upgrader is seen (GW2) |
| `verdict` | GW6 | GW6's state, never overwritten: the card falls back to it after a local Retry (startFailed) |
| `reasons` | GW6 | `[{code, key, params, message}]`, the compat reasons. **GW7 must not overwrite it**; a start failure goes into `startError` |
| `startRc` | GW7 | pkexec/wrapper exit code or null (design §4.3) |
| `startError` | GW7 (additive) | `{code, key, params, message}` of startFailed (`update.release.err.noPassword\|wrongPassword\|refused\|noWindow`, rendered in the call's language). The card also accepts `reasons[0].message` if GW7 prefers the design's field, but `startError` keeps the verdict's reasons intact |
| `logPath` | GW7 (additive, optional) | ended: the log to open; default `/var/log/dist-upgrade/main.log` (re-rooted in test mode would need this field) |
| `heldNames` | GW7 (additive, optional) | updatesFirst: names of held packages that block (design §6.4 "the card's relUpdatesFirst lists their names") |
| `version`, `codename` | GW5 (GW7 may fill them from `upgrade.json` mid-upgrade) | running/waiting are hidden without a version; the hand-over states show even when `offered` turned false |
| `allowAnyway` | GW6 | hold only: the card offers "Toch upgraden…" and then calls **`update.release.start {}`** (no `anyway` key: `update.rs`'s allowlist has none). GW7's start must therefore refuse `hold` unless `allowAnyway` is true for that codename (`compat::entry(&compat::releases(paths), codename)["allowAnyway"]`) and refuse `blocked` always; `compat::cached(shared, codename)["state"]` gives the verdict without running anything. If GW7/WU-FIX add `{anyway:true}` to the allowlist instead, the card's one call site changes (`startDialog(true)`) |
| `ubuntuPromptHeld`, `upgradeBroken`, `eol`, `supportMonths`, `notesUrl`, `released`, `helper` | GW5 | as GW5 built them |
| `compat`, `gnomeMajor`, `upgraderNl` | GW6 | see §2.2 |

Calls the card makes for GW7's states: `update.release.attach {}` (running, waiting), `update.release.start {}`
(ok/warn/hold-anyway after the start dialog), `update.install {}` (updatesFirst), `files.open {path}` (ended),
`host.restartNow()` (rebootFirst). startFailed's Retry and ended's Sluiten are page-local: the card calls nothing, so
GW7 decides when `upgrade.json`/the status file go (design §6.4 "once the card has shown it"). Where GW7 computes its
states: a `// GW7` block in `release::status` after GW6's `compat::fill` (the `view` is bound there) keeps both
steps' code apart.

## 4. Verification (all from `.wt/gw6`)

| Check | Command | Result |
|---|---|---|
| ELF = readelf | `tests/update/test_compat.sh` step 1 (`readelf -d` on `target/debug/win11-*` vs `fixtures/compat/readelf-needed.txt`) + unit test `elf::tests::our_binaries_equal_readelf` (the parser vs the same file) | 12 binaries, equal; unit test ok |
| D-G27 apt tree | `tools/heavy.sh --min-mb 1024 --tag gw6-apt -- tests/update/test_compat.sh [--record] -- target/debug/deps/win11_host-12500e7eb53cf9c1` (last run `compat-20261106-061102-…`, after every compat.rs change) | **27 passed, 0 failed**: ok → ok (50), warn → warn (ffmpeg degraded + 2 third-party; GTK t64 → t65 rename not missing), hold → hold (51; note "releases.json says GNOME 50, the archive has 51"), blocked → blocked (libarchive removed, JavaScriptCore and WebKitGTK missing), ubuntuHold → ubuntuHold, cheap → hold with `skipped:'hold'` and **0 programs started, no tree**; per full case: the system's dump (fixture `/etc/apt`) shows its Post-Invoke hooks and proxy, the tree's dump shows no hook, `Dir::Etc::parts` inside the tree, the proxy copied; the tree's sources carry each stanza's own components (`main restricted universe multiverse` and `main restricted universe`), the codename's suites, `Signed-By` = the re-rooted throwaway keyring; stub log exactly 5 × (dpkg-query -S, apt-config dump without/with the tree, apt-get -q update, apt-cache policy, apt-get -s -q dist-upgrade); strace: 20 programs, 146 execve, 0 not allowed, 0 successful connect/send with an address; no socket in the worktree; the tree deleted (asserted by the layer test) |
| Unit suite | `tools/heavy.sh --min-mb 2048 --tag gw6-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | rc 0; lib **448 passed / 18 ignored** (r3: 429/17; +19 GW6 tests, +1 ignored D-G27 test), win11-launch 16 / 1 ignored, `logs/guard-calls.log` 0 bytes |
| update unit tests | `… run-tests.sh --lib -- services::update` | 102 passed, 10 ignored |
| Bus tests | `tools/heavy.sh --min-mb 1024 --tag gw6-bus3 -- tests/update/run.sh bus-tests -- target/debug/deps/win11_host-12500e7eb53cf9c1` | **9 passed, 0 failed** (an earlier run at load 5.6: 8/9, `progress_under_the_flood_timeline` saw 5 page events in one second, the load-sensitive check GW5 also hit; alone 1/1, then the full rerun 9/9) |
| cargo check | `tools/heavy.sh --tag gw6-check -- cargo check --offline --locked -j2 --all-targets --manifest-path host/Cargo.toml` | rc 0; lib 28, lib test 26 (25 dup), win11-launch 2 (+1 test) warnings = GW5's baseline, none in GW6's files |
| rustfmt | `rustfmt --edition 2024` on compat.rs and elf.rs; `tools/rustfmt-check.sh` | **0 failing** (142 files, 111 clean, 31 listed) |
| drift | `python3 -B tools/i18n/drift_rust.py --paths host/src/services/update/{compat,elf,release,paths}.rs --warnings`; plain `--warnings` | **0 errors**; 2 / 4 warnings, all of before (GW4/GW5 file names) |
| catalogue | `PATH=…node… python3 -B tools/i18n/check_rust.py --warnings` | **0 errors**, 88 warnings (52 `[unused]`, the GW7/GW9 rows and the `update.part/feature` rows compat.rs names by `format!`, + the JSON-table apostrophes) |
| check.py | `PATH=…node… python3 -B tools/i18n/check.py` | **0 errors**, 88 warnings (all rust); check_ui 0 errors 0 warnings; dict diff 0 not in BASELINE-CHANGES.ui, 0 stale |
| Mock rows | `PATH=…node… python3 -B -m unittest -v tests/i18n/test_update_mock_rows.py` | 4 ok, 1 skipped (`rows_equal_the_design`: the catalogue decides); BaselineAdditions ok with the 44 GW6 lines |
| node | `node --check --input-type=module < F` (update-release.js, mock/update.js) | 2 × ok |
| Manifest | `python3 -B tools/ui-manifest.py`, then `--check`; `python3 scripts/verify-ui.py` | regenerated (181 files), up to date; OK |
| Dumps | `tools/heavy.sh --tag gw6-dump -- tests/i18n/run.sh --lang nl\|en --record`, then `… --lang nl\|en` (compare) | record: 25 new blocks each, **no existing block changed** (block-by-block diff: only the header count and END line); compare: nl equals `baseline-nl.txt` (27749 lines), en equals `baseline-en.txt` (27744 lines) |
| Shots | `SUFFIX=-gw6 tools/heavy.sh --tag gw6-shots -- tests/update-ui/shoot.sh <25 card ids>`; `LANGS=en SIZE=800x1400 SUFFIX=-gw6-narrow …`; `LANGS=nl THEME=dark SUFFIX=-gw6-dark … C04,C06` | 50 + 25 + 2 shots, 0 failed; reviewed (sheets in `.rt/gw6/`): the card sits under the hero like the 24H2 feature-update card, one line of text per sentence at 1280, at 800 the button wraps under the text aligned with it, links wrap, nothing clipped; the warn Expander and the start dialog (three InfoBars) fit; dark mode uses the theme tokens |

## 5. Deviations and decisions for the coordinator

1. **D1 who starts the check.** The design says when the full check may run but not who starts it. The page asks
   (`update.release.compat {}`, once per page and codename) when the snapshot says `checking` with
   `compat.needed:true`; the host refuses whatever must not run (hold, recent, offline, metered, running). With
   stonking untested nothing is ever downloaded (cheap first). Retry on `unknown` sends `{refresh:true}`.
2. **D2 ubuntuHold at read time.** The cache keeps the archive verdict; `UpgradeBroken` is applied when the snapshot is
   read, so the card follows Ubuntu's file at once. `unknown` + UpgradeBroken shows ubuntuHold (the more useful fact).
3. **D3 `compat::cached` includes the cheap hold**, so GW5's postponement works for today's case; GW5's test changed
   inside a `// GW6` block (its "not hold/blocked yet" case now uses a tested releases.json).
4. **D4 `restore_hold` in `release::status`** (Settings hosts, never dry-run) when the verdict is ok/warn for the held
   codename or another codename is offered, and after a check with ok/warn: a write inside a read, but only of our
   own two keys and only while our record says we hold them.
5. **D5 renamed libraries** count by package stem (`libgtk-3-0t64` ~ `libgtk-3-0t65`), the design's "a package that
   ships the same soname" without a Contents index.
6. **D6 failures:** every apt failure is `update.compat.apt`; an internal one (no binaries, dpkg-query, the tree) is
   `unknown` without a reason ("De controle is niet gelukt." alone). No new TSV row, so no BASELINE-CHANGES.rust-gw6.
7. **D7 the tree's own apt-config dump is checked in production too** (no apt run when it shows a hook).
8. **D8 nice/idle I/O in `pre_exec`** (no Tool::Nice/Ionice start), as GW5 did for nice; the two tools stay in the table.
9. **D9 apt's own probes.** In the layer apt runs `dpkg --print-foreign-architectures` and `ischroot -t` itself
   (read-only, no dpkg state visible there); the strace check allows exactly these argv. In production they read the
   real `/var/lib/dpkg/arch` as the user (part of R6).
10. **D10 fixture realism:** the fixture's libraries are auto-installed (`extended_states`); with all packages manual,
    apt 3's solver kept everything back instead of replacing the renamed GTK.
11. **D11 card details:** `running` is hidden while the hero shows releaseRunning (same title, body and button) and
    without a version; `waiting`'s button is standard then (one accent per page); startFailed's Retry and ended's
    Sluiten are page-local; the allowAnyway warning is the start dialog with the hold text as a warning InfoBar and
    the new key **`relAnywayStart`** "Toch upgraden" / "Upgrade anyway" (the design's relAnyway without its ellipsis;
    the only text not literally in §3.5); updatesFirst opens the start dialog after the install on this page.
12. **D12 the logo** in the mock is the glyph (the mock serves letter tiles for themed names).
13. **M01–M04** (§10's GW6 row) were built by GW5; untouched.
14. **For Robin (D-G1b class), unchanged:** the design's "deze pc"/"this PC" in relChecking/relBlocked (24H2 would say
    "dit apparaat"; the same question as D-G48's restart-row item).
15. **For GW8:** every card state is in the mock (25 ids); the harness pass needs a Settings run whose fixture offers
    stonking. `tests/i18n/MOCK-CHANGES.md` (not GW6's) may want a row for the card scenarios and the live methods.

## 6. What Robin's R5, R6 and R10 need

* **R5 (VM):** the VM build must carry a `releases.json` with stonking `tested`, `gnomeShell` = the VM extension's
  major and `allowAnyway:true`; only then does the card run the real apt tree against Ubuntu's archive (about 15 MB,
  250–400 MB RSS for apt) and show ok/warn/hold/blocked. Note in the report what the card said, the
  `~/.cache/win11-gnome/update/compat-stonking.json` record, and that `release-check/` is gone afterwards.
* **R6 (this PC):** with this build stonking holds by releases.json, so the card shows "Ubuntu 26.10 is onderweg" and
  nothing is downloaded; R6's "let the check run" becomes possible only with a build that marks the release tested
  (after GX2). Then: Settings › Update en herstel with 26.10 offered → the check runs once (unmetered) → read the
  verdict; `ls ~/.cache/win11-gnome/update/` shows `compat-<codename>.json` and no `release-check/`.
* **R10 (C14):** built as the default: Ubuntu's own prompt is postponed only on the click "Ook de melding van Ubuntu
  uitstellen" (hold/blocked cards), "Ongedaan maken" restores it, and it is restored by itself once the verdict is
  ok/warn or another release is offered. Robin chooses: keep click-only, make it automatic while on hold (one call in
  `compat::fill`'s hold branch), never offer it, or switch to "Alleen LTS-versies" (R4).
