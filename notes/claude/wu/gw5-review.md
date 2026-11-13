# GW5 review: release detection, the Prompt choice, attach, the system-file step (adversarial, 2026-11-06)

Reviewer: an independent agent. I did not write GW5. Worktree `.wt/gw5` (branch `r3/gw5`, HEAD 18c8ffbe, nothing
committed). I changed no product file. I wrote only this note and scratch files under `.rt/review-gw5/`:
* logs;
* a merged scratch copy `.rt/review-gw5/root/` (vendor/ui, tests/i18n with the two GW5 side files appended,
  tests/update-ui, tools/i18n, host/i18n), so the M states could be dumped and shot without editing
  `tests/i18n/surfaces.json`;
* helper probes on temporary etc dirs.

Every heavy command went through `tools/heavy.sh` with a `gw5-rv-*` tag. I ran no win11-* binary, gnome-shell,
pkexec, check-new-release, apt, gsettings or dconf on the real system, and I signalled no process.

One slip to report: in my own bubblewrap box (a copy of test_helpers.sh's layer: `--unshare-all`, tmpfs
`/run/dbus`, `/run/user` and HOME, a logging pkexec stub first on PATH, `strace -f`), a loop over refused argument
lists passed an empty string unquoted. So `install-system.py` ran once with no argument, the install path, instead
of `--dry-run`. It reached only the stub (logged, exit 97). strace shows no other execve and no connect/sendto, and
the only writes were its own stage under the box's private `XDG_RUNTIME_DIR` (removed) and the stub's log. Nothing
outside `.rt/review-gw5/box/` changed, but it was not a `--dry-run` call. That run is what shows m3 below.

**Verdict: fix-needed (one major, six minors, eleven notes, no blocker).** Safety holds: I found no path from GW5's code,
helpers, scripts or tests to the real system bus, the live session, real `/etc` or dconf, a working pkexec, or the
network. Every done-when check I re-ran passes with the step's numbers, and the merged mock-rows test passes
(4 ok, 1 skipped). The Prompt row fits in nl and en at 1280 and 800 px and matches the Windows 11 card look.

* **M1 (major):** nothing calls `update.release.check`. The design's "only on a click of Controleren op updates" has
  no click path yet, and the fix lies outside GW5's files.
* **m1–m6:**
  * m1: a Prompt change can be a silent no-op.
  * m2: a stale motd stamp can offer the current release.
  * m3: the installer's failure text points at output it never prints.
  * m4: installer wording (Dutch "Settings" for Instellingen; awkward English).
  * m5: `promptShInfo` appears only while the password dialog is open, an interpretation the step did not list.
  * m6: a latent harness path through the production tool table, to close before M1 is wired.

## 1. Checks I ran (from W's root)

| Check | Command | Result |
|---|---|---|
| Unit suite | `tools/heavy.sh --min-mb 2048 --tag gw5-rv-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | rc 0; lib **424 passed / 17 ignored**, win11-launch **16 / 1 ignored**, `logs/guard-calls.log` 0 bytes (= claim) |
| cargo check | `tools/heavy.sh --tag gw5-rv-check -- cargo check --offline --locked -j2 --all-targets` | rc 0; lib 28 warnings, lib test 26 (25 dup), win11-launch 2 (+1 test); none in `services/update/` (= claim) |
| Bus tests (regression, not in done-when) | `tools/heavy.sh --min-mb 1024 --tag gw5-rv-bus -- tests/update/run.sh bus-tests -- target/debug/deps/win11_host-12500e7eb53cf9c1` | **9 passed, 0 failed** (GW5 changes `compose`'s release object and the release.* writes those tests call) |
| GW5 tests in the strict layer, traced | `tools/heavy.sh --tag gw5-rv-exec -- tests/update/run.sh exec --no-strace -- sh -c 'strace -f -e trace=execve,connect … BIN services::update::release services::update::admin'` | 16/17; the failure is n9 (uid mapping). Programs started: only the fixture's stubs (`check-new-release` 5, `pkexec` 12, `release-upgrades-helper` 3, `sh` 2) and the helper's coreutils on the temp etc (awk, chmod, chown, id, mv, sync); **no execve of `/usr/bin/pkexec`, `check-new-release`, `/usr/libexec/win11-gnome/*`, dconf, gsettings or apt**; connect() only nss/nscd ENOENT and the layer's own bus; layer stub log 0 calls, fake CallCount 0 |
| Helpers, policies, install-system.py | `W11_TEST_TMP=$W/.rt/review-gw5 tests/update/test_helpers.sh` | rc 0, **124 passed, 0 failed** |
| install-system.py, my own layer | bwrap `--unshare-all`, tmpfs `/run/dbus` `/run/user` HOME, pkexec stub, `strace -f -e trace=%file,%network,%process`: `--dry-run`, `--dry-run --remove`, en `--dry-run`, refusals `--dry-run=1`, `--`, `-`, `--dry-run --dry-run --remove`, `--Dry-run` | dry runs rc 0, print `<stub> /bin/sh -c '<INSTALL_SCRIPT>' win11-gnome-system-install <stage> <4 sha256>` (remove: `… win11-gnome-system-remove`) and the checksums; refusals rc 2 with the usage line; **0 connect/sendto**; every write inside the stage under the box's `XDG_RUNTIME_DIR`; stage removed; plus the slip above |
| Installer | `tests/installer/run.sh` (REAL cases, its own heavy.sh) | **37 tests OK** |
| Package | placeholders for the 12 binaries, `TMPDIR`/`WIN11_PACKAGE_DIR` under `.rt/review-gw5/pkg`, `tools/heavy.sh --tag gw5-rv-package -- python3 -B scripts/package.py $W` | rc 0; tarball has `host/data/` with both helpers 0755 and both policies 0644 (tarball removed) |
| drift / ratchet | `python3 -B tools/i18n/drift_rust.py --warnings` | **0 errors**, 3 warnings (technical file names, one GW5's `gschemas.compiled`) |
| catalogue | `python3 -B tools/i18n/check_rust.py --warnings` | **0 errors**, 68 warnings (60 `[unused]` GW6/GW7 rows + 8 old); none for GW5's rows |
| pairs, UI | `check_pairs.py --warnings`; `check_ui.py --warnings`; `check_ui.py --dict-diff tests/i18n/dict-nl-3b898e6.tsv --changes tests/i18n/BASELINE-CHANGES.ui --quiet` | 0/0 (install-system.py's pairs included); 0 errors, 50 glossary warnings, none in GW5's files; 270 added, **0 not in BASELINE-CHANGES.ui** |
| node, manifest | `node --check --input-type=module` on update-release.js and mock/update.js; `tools/ui-manifest.py --check`; `scripts/verify-ui.py` | 2 × ok; up to date (181 files); OK |
| Mock rows | `python3 -B -m unittest tests/i18n/test_update_mock_rows.py` in W / in the merged copy | W: 1 failure (side file not merged, as reported); merged: **4 ok, 1 skipped** |
| Dumps | merged copy: `tools/heavy.sh --tag gw5-rv-dump -- .rt/review-gw5/root/tests/i18n/run.sh --lang nl\|en --only settings.update,settings-update.H01,…H03,…H25,settings-update-prompt.* --out …` | against `tests/i18n/baseline-{nl,en}.txt`: settings.update, H01, H03 **−0 +6**, H25 −0 +5 (only the Prompt row; no existing line changed in nl or en); 6 new `settings-update-prompt.*` blocks; no Dutch word in the en blocks (`dutch-words.txt`) |
| Shots | merged copy `tests/update-ui/shoot.sh` (its nonet layer): settings.update + M01, M01b, M02, M03, M04, M04b in nl/en 1280; the six in en 800; M01, M04 nl dark | 14 + 6 + 2 shots, 0 failed; looked at all (§4) |
| Ownership | `git -C W status --short`, `git -C W diff --stat` | only owned files: 10 modified (1783+/52−) + host/data/, install-system.py, fixtures/release/, test_helpers.sh, BASELINE-CHANGES.rust-gw5, the two side files, wu/gw5.md; `vendor/ui-sha256.json` regenerated as allowed; unchanged by my runs |

## 2. Conformance (task, design §6.1, §6.3, §7, §10 GW5 row)

Present and as designed:
* Prompt precedence (main file, then sorted `.d/*.cfg`, the last key wins, the deciding file named, override
  disables the row). It matches `MetaRelease.py` 149-157 and `_parse_cfg`, apart from m1's edge cases.
* The meta-release parse: required keys, `parsedate` forms, the current release listed, the first later supported
  stanza.
* The newer copy wins; the stamp counts by version only; distro-info `eol`/`supportMonths`; released-but-not-offered.
  The fixture "today" follows design §1.2.
* The check (click-only write, 24 h, metered/offline/never skips, `LANG=C`, nice 10, 60 s) runs through the tool
  table, a stub only in tests.
* `ubuntuHold`: a keyfile round trip of both keys that restores only our values, `ubuntuHoldPrevious`.
* The trust rule is `cleanup.rs:679-695`'s. The pkexec runner: 126/127 count only without a marker.
* The `/bin/sh` fallback runs the helper text itself (D2). `attach_argv`.
* The helpers: `set -u`, fixed PATH, umask 022, exact arguments, temp + chown + chmod + sync + mv, ASCII markers,
  `WIN11_HELPER_ETC` only when not root.
* The policies: texts and defaults as §7, attach without `_keep`.
* `install-system.py` is standalone, refuses root and other arguments, uses a 0700 runtime-dir stage, one pkexec
  call, and is bilingual by C1. D3 adds the checksums and the root-owned copy: a real hardening.
* The hint lines (D4: TTY only); package.py.
* The Prompt row with all §3.6 texts. M01–M04 (+M01b, M04b) are mocked.

Missing or different: M1 (no click path), m5 (InfoBar timing), D1–D10 as listed in gw5.md §5 (I agree with D1, D2,
D3, D4, D7, D8, D9, D10). §6.3's argv differs by `set-prompt` (D2, fine).

## 3. Findings (most severe first)

### M1 (major, owner: coordinator → R3-UI or GW6) `update.release.check` has no caller

Design §6.1(3) wants check-new-release to run on a click of "Controleren op updates". The host side exists, but
`update.js:525` is `checkFor: { … run: (b) => write('update.check', {}, b) }`. A grep of vendor/ui, extension and
host/src finds `update.release.check` only in `mock/update.js:667`, the method table and the dispatch. So the click
never refreshes the release offer. The offer still arrives later through Ubuntu's own check-new-release-gtk and the
motd timer. The step reports this itself (gw5.md §5 D5), but it is a missing requirement of the feature and nobody
is assigned yet.
**Fix:** one line in `update.js` (`checkFor` also calls `call('update.release.check', {})` without waiting; a
`skipped` answer is silent), assigned to R3-UI or GW6. Land it only after m6 is closed.

### m1 (minor) A Prompt change can be a silent no-op; the helper's reader is not configparser's

`set_prompt` (release.rs:623-626) answers `{prompt: after.value, changed: true}` after the helper's `@@ok`, without
checking that the file now says what was asked. The helper's awk (release-upgrades-helper:69-96) recognises
`[DEFAULT]` only as the whole trimmed line and does not strip `\r`. My probes on a temp etc, each read back with
Python's configparser as `_parse_cfg` does:
* **CRLF file** (`[DEFAULT]\r\nPrompt=normal\r\n`, set lts): `@@ok prompt=lts`. A second `[DEFAULT]\nPrompt=lts` is
  appended. Ubuntu then refuses the file (DuplicateOptionError) and behaves as normal. `config_prompt` reads the
  first key (normal), so the page silently shows "Elke nieuwe versie" again with no error.
* **`[DEFAULT] ; main`:** the same DuplicateOptionError, ignored file, `@@ok`.
* **A file Ubuntu already refuses** (`Prompt=never` before any section): `@@ok`, and Ubuntu still ignores it.
* **Reverse:** `config_prompt` reads an indented `  Prompt=never` after another key as Prompt. configparser reads it
  as that key's continuation (Ubuntu: normal).
All are rare in `/etc`, but the result is a password prompt that changes nothing, with no message.
**Fix:**
* release.rs: after the helper, `after.value != want` (no override) → `update.release.err.promptFailed`.
* The helper: strip a trailing `\r` before matching. Treat `[DEFAULT]` followed by anything as the DEFAULT header
  (configparser's SECTCRE). Re-scan the temp file before `mv`: exactly one Prompt key in DEFAULT, equal to the
  value, else `@@fail parse`, and the old file stays.
* `config_prompt`: treat a file Ubuntu refuses (a duplicate key in a section, a key before any section) as having no
  Prompt.
* Add the CRLF case to `test_helpers.sh`.

### m2 (minor) A stale stamp that names this release's point version is offered

`detect`'s stamp path (release.rs:435-446) compares `version_key("26.04.1 LTS")` = [26,4,1] with `VERSION_ID`
"26.04" = [26,4]. The point version counts as later, `short` = [26,4] finds `resolute`, and the offer becomes the
current release (`offered:true, codename:resolute`). It can happen when a stamp written on 25.10 ("New release
'26.04.1 LTS' available.") outlives the upgrade and is newer than both copies. Whether Ubuntu clears the stamp on
upgrade is unverified. If it does not, GW6's card would offer "Ubuntu 26.04.1 LTS" on 26.04.
**Fix:** compare only the first two components, and require the codename to differ from `VERSION_CODENAME`. Add a
fixture `stamp/current-point-en`.

### m3 (minor) install-system.py: "see above what was done" but nothing is shown

After pkexec, `outcome()` keeps the `@@ok` paths only for success. The `failed` branch (install-system.py:284-286)
prints `T.failed`: "Het is niet gelukt ({code}). Kijk hierboven wat al wel is gebeurd, …" / "That didn’t work
({code}). See above what was done already, and try again." Above it there is only the intro list and "Ubuntu now
asks for your password once." (the slip run: `That didn’t work (rc 97). See above what was done already, and try
again.`). After a partial install (`fail install` on the third file) the user cannot see that two files are already
installed.
**Fix:** print `Installed:`/`Removed:` for every `@@ok` line before the failure text (parse them in `outcome()` for
every key), or drop the sentence.

### m4 (minor) installer wording (new texts, no baseline involved)

* `T.doneInstall` nl: "Klaar. **Settings** gebruikt de nieuwe bestanden meteen; …". The app is "Instellingen" in
  Dutch (glossary `Instellingen → Settings`). Fix: "Klaar. Instellingen gebruikt de nieuwe bestanden meteen; je hoeft
  niet opnieuw aan te melden."
* `T.intro` en: "With them, Ubuntu says what your password is for at New Ubuntu versions and at Show the upgrade
  window:". This is not idiomatic. Suggest: "Win11 installs four system files, so that Ubuntu’s password prompt says
  what it’s for when you use ‘New Ubuntu versions’ and ‘Show the upgrade window’:". For nl, quote the two names too:
  "… bij ‘Nieuwe Ubuntu-versies’ en ‘Venster weer openen’ …".
* `T.dryRun` en: "and no password is asked" → "and you won’t be asked for a password".
* Optional: `promptOverride` "This setting is controlled by {file}." is the design's text. Windows 24H2 says
  "managed by" ("Some of these settings are managed by your organisation"), so "This setting is managed by {file}."
  would be closer.

### m5 (minor) `promptShInfo` appears only while the change waits

Design §3.6: "helper missing: InfoBar `promptShInfo`". update-release.js:114 shows it only while `wanted` is set,
that is while GNOME Shell's system-modal password dialog dims the screen. So the explanation of "/bin/sh" is never
visible before the user decides, and behind the dim layer while the dialog is up (nl-dark and light shots of M04).
This interpretation is not in gw5.md §5.
**Fix:** either show it whenever `release.helper.prompt === false` (an info line under the row, or the InfoBar,
closable), or list the interpretation for the coordinator and GW8, so M04's expected screenshot is agreed.

### m6 (minor, latent) the production tool table bypasses the harness' PATH stubs

Outside test mode `Tools::path` returns the absolute table paths (paths.rs:350-353). In a GNOME harness run without
D-G44's update seams (`WIN11_UPDATE_TOOL_DIR`/`FIXTURE_ROOT`, not in harness.py yet), a scenario that changes the
Prompt row execs the real `/usr/bin/pkexec`. That is inert there: no_new_privs, empty `/run/dbus`, so pkexec stops at
"must be setuid root" and the page shows wrongPassword. Once M1 is wired, a "Controleren op updates" click would exec
the real `/usr/lib/ubuntu-release-upgrader/check-new-release`. There is no network in the layer, but the rule says
agents never run it. Neither reaches the real system today, and nothing calls `release.check`. Still, the harness'
`STUBS` (check-new-release, pkexec) give a false sense of coverage here.
**Fix:** wire M1 only after R3-NATIVE gives every harness Settings run the update seams. Add a harness self-test
assertion that no execve of `/usr/bin/pkexec` or `check-new-release` happens. GW8 must run every M state with the
seams.

### Notes

* **n1:** Paths has two `// GW5` spots: the fields and their initialisers in `rooted()`. That is needed for a struct
  (GW4 did the same), so "one block" is met in spirit.
* **n2:** D6's additive fields (`name, upgradeBroken, source, currentLts, lastCheck, checkError`, `changed`,
  `{started:false, skipped}`) need `api-methods.json` docs (WU-FIX) and a `tests/i18n/MOCK-CHANGES.md` row for the
  Prompt scenarios and the live setPrompt (not GW5's files).
* **n3:** `lastRun` is written after a failed or stub-less check too (release.rs:595). A failed check therefore
  blocks a new click for 24 h. That is what §6.1's "at most once per 24 h" says, but a failed run could use a shorter
  backoff (for example 1 h).
* **n4:** a 0-byte newer meta-release copy parses as zero stanzas, wins, and hides an older valid copy.
  `MetaRelease` deletes empty copies before it downloads. Suggest: no stanza = does not parse.
* **n5:** the row stays enabled while Ubuntu's upgrader runs (H25). A change then gets `update.guard.upgrade` in the
  error InfoBar. Optional: disable it while `phase === 'releaseRunning'`.
* **n6:** `install-system.py` takes pkexec from PATH (`shutil.which`). The stub test needs this, and the dry run shows
  the resolved path, so a user can check it. Optional: prefer `/usr/bin/pkexec` unless a test variable is set.
* **n7:** `test_helpers.sh` checks D4 (hint only on a terminal) by grepping the source, not by running it.
  Acceptable.
* **n8:** `update.release.setPrompt` holds its HTTP request (one tiny_http thread) and the page's fetch, which has no
  timeout, for as long as the password dialog is open. That is the same pattern as `cleanup.rs`'s pkexec, so it
  probably works. R4 confirms it live.
* **n9:** `admin::tests::the_trust_rule_is_cleanup_rs_rule` (admin.rs:244) asserts on the host's real `/etc/passwd`
  being root-owned. Inside `tests/update/run.sh`'s layer (a user namespace maps uid 0 to nobody) it fails, 16/17
  there. It passes in run-tests.sh, the done-when sandbox. Suggest: skip that assertion when `/etc/passwd`'s uid is
  not 0.
* **n10:** R4's last step (`cmp` silent after switching back to normal) holds for Ubuntu's default file
  (test_helpers.sh: byte-identical). If Robin's file lacks a final newline, the helper adds one. Say so in R4.
* **n11:** M02's mock sets `released:null`. On a real interim release with lts, `released` lists every version
  (lts_only is false), so the M02 shot may differ from GW8's real state by the `relNotOffered` line. Harmless for the
  mock.

## 4. Look (screenshots I viewed)

Shots: `.rt/review-gw5/root/tests/update-ui/shots/{nl,en,en-narrow,nl-dark}/`; contact sheets:
`.rt/review-gw5/sheet-{nl,en,narrow,dark}.png`.
* **Placement:** the Prompt card is the first item of Meer opties/More options, Windows' "Get the latest updates"
  slot. It uses the standard icon, title, secondary description and a 200 px ComboBox on the right, with the same
  radius, padding and spacing as the history/advanced cards.
* **M01:** three description lines ("Elk half jaar…", the GNOME line, "Ubuntu 26.10 is uitgebracht…" / "… is out…").
  Heavier than a typical Windows card, but not clipped in nl or en.
* **M01b:** "Nooit melden" / "Never notify" with no description (as designed).
* **M02:** shows the interim line.
* **M03:** the path line, with the ComboBox greyed.
* **M04:** an info InfoBar below the card, one line at 1280 and three at 800, with the ComboBox greyed on the chosen
  value.
* **M04b:** a red error InfoBar with a close button.
* **800 px en:** D9's `flex-basis: 0` keeps the text beside the icon, and the ComboBox wraps below it, as in Windows'
  narrow layout. The override path breaks at "release-/upgrades.d". Nothing is clipped or overflows.
* **Dark:** tokens are correct, contrast is fine.
* **Dutch:** existing Update page lines are unchanged in both dumps.

## 5. Language

* **New UI en (§3.6 texts):** en-GB, sentence case, the typographic apostrophe ’, glossary-clean (check_ui: no
  warning in GW5's files).
* **New host row `update.release.err.notHeld`:** "This is only possible while Win11 Shell isn’t ready for the new
  Ubuntu version yet." is fine. Its Dutch is new, listed in BASELINE-CHANGES.rust-gw5.
* **Policies:** the design's texts exactly (test_helpers.sh checks them with xml.etree).
* **Installer:** see m3 and m4.
* **Errors** are matched on code/key: `failure_key` maps to `update.release.err.*`, the page shows the host message,
  and the mock's `update.release.err.noPassword` equals update.tsv.
* **"For Robin" items:** none in GW5's area, as gw5.md says.
