# GW5: release detection, the Prompt choice, attach, the system-file step (2026-11-06)

Step GW5 of `notes/claude/windows-update-gnome.md` §10 (design §3.5/§3.6 Prompt-row texts, §3.7, §4.3
`update.release.*`, §6.1, §6.3, §6.4 attach only, §7, §11 R2/R4), in the worktree `.wt/gw5` (branch `r3/gw5`, from
18c8ffbe). Start state checked: `git status --short` empty, HEAD 18c8ffbe. Nothing was committed. No bus, PackageKit,
apt, pkcon, do-release-upgrade, check-new-release, update-manager, pkexec, sudo, gsettings or dconf was run on the
real system, no win11-* binary or gnome-shell, no network: every program of the service ran as a stub from a marked
tool directory, every GSettings write went to a keyfile backend in a fixture root, the helpers and the root scripts
ran as Robin on temporary etc/root directories, `scripts/install-system.py` ran only with `--dry-run` or with
arguments it refuses (inside bubblewrap, with a pkexec stub), every Chrome run inside `tests/update-ui/nonet.sh`'s
layer and every heavy command through `tools/heavy.sh`. Today's meta-release shape comes from design §1.2 (newest
`resolute 26.04.1 LTS`, no stonking, an empty stamp), never from the real `/var` or `~/.cache` files. Local tools run
outside a sandbox: `glib-compile-schemas` (writing only `tests/update/fixtures/release/schemas/`), `python3 -I`
(fixture generation), `sh`/`awk`/coreutils inside the helpers on temp directories. Scratch files are in `.rt/gw5/`.

## 1. Files

| File | What |
|---|---|
| `host/src/services/update/release.rs` | Prompt detection, the offer (stamp + both meta-release copies, parsed like `MetaRelease.parse`), distro-info dates and "released but not offered", the snapshot's `release` object, `update.release.check` (click only, daily, through the tool table), `update.release.setPrompt`, `update.release.ubuntuHold` (keyfile round trip, restores only our values); 13 unit tests |
| `host/src/services/update/admin.rs` | the trusted-helper check (cleanup.rs's rule), the pkexec runner (helper or `/bin/sh -c` with the same script, 126/127/markers), `attach_argv` for GW7, `helpers()` for the snapshot; 4 unit tests and the shared test helpers (`pkexec_stub`, `wire_settings`, fixtures) |
| `host/src/services/update/paths.rs` | only the `// GW5` lines: `Paths.user_cache` / `user_home` (production `glib::user_cache_dir()` / `home_dir()`; with a fixture root `<root>/home/.cache`, `<root>/home`) |
| `host/i18n/update.tsv` | one new row through `extract_rs.py add`: `update.release.err.notHeld` (§5 D1) |
| `tests/i18n/BASELINE-CHANGES.rust-gw5` | its `key:` line (a new Dutch text; no existing Dutch changes) |
| `host/data/release-upgrades-helper` | the Prompt helper (`/bin/sh`, `set -u`, fixed PATH, umask 022, `set-prompt lts\|normal\|never` exactly, temp file + chown + chmod + sync + mv, `@@ok`/`@@override`/`@@fail`, `WIN11_HELPER_ETC` only when not root; ASCII only) |
| `host/data/release-upgrade-attach` | `exec /usr/bin/screen -d -r ubuntu-release-upgrade-screen-window`; no arguments, refuses a non-root caller |
| `host/data/io.github.win11shell.gnome.release-upgrades.policy` | auth_admin / auth_admin / auth_admin_keep, `exec.path` the helper, the design's en + `xml:lang="nl"` texts |
| `host/data/io.github.win11shell.gnome.release-upgrade-attach.policy` | auth_admin ×3 (no keep), `exec.path` attach, the design's texts |
| `scripts/install-system.py` | new, standalone (§2.5) |
| `scripts/install.py`, `scripts/uninstall.py` | one hint line each (`install.system_hint`, two new `T` pairs in install.py, the `SYSTEM_FILES` list); no flag |
| `scripts/package.py` | the tarball must carry the four `host/data` files (`install.t("badTable")` otherwise), helpers 0755 |
| `tests/update/fixtures/release/**` | Ubuntu's default `release-upgrades`, the `ubuntu-advantage-upgrades.cfg` without Prompt, `distro-info/ubuntu.csv` (this machine's header and last rows), meta-release copies (`today`, `today-lts`, `stonking-supported`, `-unsupported`, `-broken`, `broken-file`), stamps (empty, en, nl, LTS, "not supported", and since the fix round `current-point-en`), the update-manager schema (2 keys, same id/path/types/defaults) + `gschemas.compiled`; fix round: `cfg/` (39 configparser corpus files + `expected.tsv`, §6.1 m1) |
| `tests/update/test_helpers.sh` | new: 124 checks (§3); 131 after the fix round (§6.3) |
| `vendor/ui/js/apps/settings/pages/update-release.js` | the Prompt row (`promptRow`) and its makeT dictionary (11 keys); `releaseCard` stays GW3's placeholder for GW6 |
| `vendor/ui/js/mock/update.js` | only Prompt-row parts: scenarios M01, M01b, M02, M03, M04, M04b, a live `update.release.setPrompt`, `currentLts` in the base release object, the host row `update.release.err.noPassword` (= update.tsv), header lines |
| `vendor/ui-sha256.json` | regenerated with `tools/ui-manifest.py` (2 files; `ui-patches.json` unchanged) |
| `notes/claude/surfaces-additions-gw5.json` | 6 surfaces `settings-update-prompt.M01…M04b` + their comment line, for `tests/i18n/surfaces.json` |
| `notes/claude/baseline-additions-gw5.txt` | the 11 new nl keys, for `tests/i18n/BASELINE-ADDITIONS.ui` |

Not touched: `update.rs`, `tests.rs`, `bus_tests.rs`, every other `update/*.rs`, `api-methods.json`,
`settings-defaults.json`, `update.js`, CSS, `tests/i18n/*` (except the new BASELINE-CHANGES.rust-gw5), frozen files.

## 2. What was built

### 2.1 Detection (`release::detect`, §6.1)

* **Prompt**: `release-upgrades`, then `release-upgrades.d/*.cfg` in byte order; the last file with a `Prompt` key in
  its `[DEFAULT]` decides. Each file is read as Python 3.14's configparser reads it (`config_prompt`, a port of
  `_read_inner`; fix round m1): case-insensitive key, `=` or `:`, the whole rest of the line is the value (no inline
  comments, so `Prompt=lts # x` is normal, as Ubuntu reads it), deeper-indented lines continue it (comments and
  blank lines in between do not end it), `\r\n`/`\r`/`\n` line ends, and a file configparser refuses (a key twice,
  a key before any section, a junk line, an empty key, a section twice) is ignored, as `_parse_cfg` ignores it.
  `never`/`no` → never, `lts` → lts, else normal. `promptFile` is the system path of the deciding file; `override` is
  set when a `.d` file decides.
* **Offer**: the copy Ubuntu reads for this Prompt (`meta-release-lts` only for lts on an LTS release) in
  `$XDG_CACHE_HOME/update-manager-core/` and `/var/lib/update-manager/`; the newer copy that parses wins. A stanza
  without Dist/Version/Supported/Date (or one that does not parse) refuses the file, as `MetaRelease.parse` does.
  Dates as `email.utils.parsedate` reads them (3-letter or full English months, day/month in either order, 2-digit
  years, the zone ignored). The current release must be listed; the first later stanza with `Supported: 1` is the
  offer (Version, Name, ReleaseNotesHtml → `notesUrl`, UpgradeBroken → `upgradeBroken`).
* **Stamp**: only when the copy has no offer, only when the stamp is not older than that copy, only a release
  (year.month) later than `VERSION_ID` (`\d\d.\d\d(.\d)?( LTS)?`, any language; a stale "26.04.1 LTS" on 26.04 is no
  offer, fix round m2); the codename then comes from distro-info and is never the current one. An empty copy is no
  copy (fix round n4).
* **distro-info** by header name: the offer's `eol` and `supportMonths` (release → eol: 26.10 = 9); `released` = the
  newest release that is out, still supported and later than the current one (LTS only for lts on an LTS release).
  Today in the fixture: `offered:false`, `released:{26.10, stonking}` → "Ubuntu 26.10 is uitgebracht; …".
* **Snapshot** (`release::status`, additive to §4.3): `offered, released, version, codename, name, eol,
  supportMonths, notesUrl, upgradeBroken, source ('cache'|'stamp'), prompt, promptFile, override, currentLts,
  helper{prompt, attach}, checkedAt (the deciding copy's mtime), lastCheck, checkError, ubuntuPromptHeld`; `state`
  stays `none` (GW6's verdict), `gnomeMajor`/`reasons` GW6, `startRc` GW7. Files only: no program, no bus. Unit tests
  read release files only below a fixture root (`reads_files`). `ubuntuPromptHeld` opens update-manager's settings only
  in a `settings` host and only while a hold record exists.

### 2.2 `update.release.check {}` (§6.1 step 3)

A write method (so never in dry-run, forwarded by a mirror, refused while the upgrader runs; status and every read run
no program). `{started:false, skipped}` for Prompt=never, offline (NM connectivity none), metered without
`update.metered`, a run in the last 24 h, or a run in flight; else `{started:true}` and, on its own thread,
`check-new-release -q` from the tool table (a stub in test mode; a missing stub is an error, never PATH) with `LANG=C`,
nice 10 (in the child, no extra program), stdin/stdout/stderr null, 60 s, `XDG_CACHE_HOME`/`HOME` set to the paths the
detection reads (and the proxy variables in production). rc 0/1 fine, anything else
`update.release.err.checkFailed` (`release.checkError`, rendered per call). `release.json` keeps `lastRun`, `rc`,
`error`, `newRelease`; then `update.changed` with the next `rev` (not written: the snapshot's `rev` is the pk
thread's; the page reloads because the event's `rev` differs from the snapshot's).

### 2.3 `update.release.setPrompt {prompt}` (§6.3) → `{prompt, override, changed}`

No password for nothing: a `.d` file decides (answer: the effective value and the file), or the file already says it
(no key there counts as normal; so does a file Ubuntu ignores). Otherwise `admin::run`: `pkexec <helper> set-prompt <v>` when the helper is trusted
(regular file, owner root, no g/o write, parent the same; in test mode the stub directory's owner), else
`pkexec /bin/sh -c '<the helper file's text>' win11-gnome-prompt set-prompt <v>` (§5 D2). Success needs
`@@ok prompt=<v>`, no `@@fail`, rc 0; 126/127 without a marker → `update.release.err.noPassword` (`auth_cancelled`) /
`.wrongPassword` (`auth_failed`); anything else `.promptFailed`. Cleared environment, stdin null, 10 minutes. Then
the file is read again and `update.changed` goes out; unless a `.d` file now decides, the main file must now read as
`<v>` (configparser's way), else `.promptFailed` (fix round m1: never a silent no-op).

### 2.4 `update.release.ubuntuHold {on}` (§4.12 C14) → `{on, codename}`

On: only for the offered codename while `compat::cached(<codename>).state` is `hold` or `blocked` (GW6; until then
always refused with the new `update.release.err.notHeld`). Writes `check-new-release-ignore = <codename>` and
`check-dist-upgrades = false`, keeping `settings.update.ubuntuHoldPrevious {ignore, checkDist, codename}` (null = no
user value) once per codename; a new codename first restores the old hold. Off (`restore_hold`): each key goes back
(set or reset) only while it still holds our value; the record is cleared. No schema → `update.release.err.ubuntuHold`.
GSettings: default source/backend in production, only the fixture's schemas + keyfile in test mode (never dconf);
`gio::Settings::sync()` in production only.

### 2.5 Helpers, policies, `install-system.py` (§7)

* Prompt helper: arguments checked first (exactly `set-prompt` + one enum value), then root (`WIN11_HELPER_ETC` only when
  not root, absolute; the file then belongs to the user), then the directory/file (no symlink, a regular file or
  none), a stale temp file removed (a symlink there is removed, never followed). One awk program follows
  configparser's reader line by line (fix round m1) in three modes: `rewrite` keeps every other line, turns the first
  Prompt key of `[DEFAULT]` into `Prompt=<v>` (its indentation and CRLF kept) and drops its continuation lines and a
  second Prompt key there; without a key, `Prompt=` follows the last line of `[DEFAULT]` (blank lines before the next
  section stay after it; the next header's indentation keeps that header a header), or a new `[DEFAULT]` ends a file
  without one. `check` reads the temp file back: only when Ubuntu would read exactly `Prompt=<v>` from it (no
  configparser error, one key) does it go on to chown/chmod/sync/mv, else `@@fail parse` and the old file stays; a
  lone CR or a Unicode space outside a comment line (Python splits or strips there, awk cannot) always fails `check`.
  `probe` names the `.d` files that set Prompt (`@@override`); overrides are never changed. The corpus
  `tests/update/fixtures/release/cfg` (39 files: configparser's verdict, the helper's outcome) is checked against
  python3 by test_helpers.sh and against `config_prompt` by a unit test.
* `install-system.py [--dry-run] [--remove]`: refuses root (before anything), any other argument (each flag at most
  once), a missing source file, an `XDG_RUNTIME_DIR` that is not a 0700 directory of the user, a missing pkexec.
  Messages in Win11's language (C1, settings.json). It prints what it installs and why, stages the four files
  (fixed names, `O_EXCL|O_NOFOLLOW`, 0755/0644) in `mkdtemp(dir=$XDG_RUNTIME_DIR)`, runs one
  `pkexec /bin/sh -c '<INSTALL_SCRIPT>' win11-gnome-system-install <stage> <4 sha256>` (§5 D3), and removes the
  stage. The root script accepts only the four names, copies each to its own `mktemp -d`, checks the checksum, creates
  only missing directories, installs with `install -o root -g root -m 0755|0644`, prints `@@ok <path>`…`@@done`.
  `--remove`: `pkexec /bin/sh -c '<REMOVE_SCRIPT>' win11-gnome-system-remove`, exactly the four paths and
  `/usr/libexec/win11-gnome` when empty. `--dry-run` prints the command (`shlex.join`, copy-pastable) and the
  checksums, asks nothing, installs nothing. Both scripts honour `WIN11_SYSTEM_ROOT` only when not root (tests).
* `install.py` prints, as its last line, `Optioneel, één keer met je wachtwoord: python3 <path>/install-system.py
  installeert de systeembestanden voor Nieuwe Ubuntu-versies en Venster weer openen.` when any of the four files is
  missing; `uninstall.py` prints `De systeembestanden van Win11 staan er nog; verwijder ze met: python3 <path> --remove`
  while any is there. Both only on a terminal (§5 D4).

### 2.6 The Prompt row (`update-release.js`, §3.6; M01-M04)

A SettingsCard (icon `sparkle`, in Windows' "Get the latest updates" place, first item of Meer opties) with a
200 px ComboBox: "Alleen LTS-versies", "Elke nieuwe versie", and "Nooit melden" only while the file says never. The
description: `promptLtsDesc` or `promptNormalDesc` + `promptNormalGnome`; then `promptLtsInterim` (lts while
`currentLts` is false), `relNotOffered` (not offered, a release out, prompt not never), `promptOverride` (the
ComboBox disabled; disabled also while Ubuntu's upgrader runs, fix round n5). Without the root helper the InfoBar
`promptShInfo` stands under the row before and during a change (design §3.6 "helper missing", fix round m5). A change
calls `update.release.setPrompt`; a refusal is an error InfoBar with the host's message (closable) and the ComboBox shows the file's value
again after the reload; a dry run gets the usual toast; a mirror waits up to 10 min for `update.reply`. Hidden without
a release object or with `prompt` null (no `/etc/update-manager`). The text block gets `flex-basis: 0` inline: in a
narrow card (components.css's 476 px container rule) the long description otherwise took a line of its own under the
icon (seen in the 800 px en shot); a wide card lays out as before. Mock states: M01 normal (today), M01b never, M02
lts on 25.10, M03 a `.d` override, M04 the `/bin/sh` InfoBar (the surface picks "Alleen LTS-versies", the mock keeps
the call pending like Ubuntu's open dialog), M04b a dismissed dialog (noPassword).

## 3. Verification (from `.wt/gw5`)

| Check | Command | Result |
|---|---|---|
| Build | `tools/heavy.sh --tag gw5-check -- cargo check --offline --locked -j2 --all-targets` | rc 0; lib 28 warnings, win11-launch 2 (= baseline), none in `services/update/` |
| GW5 unit tests | `tools/heavy.sh --min-mb 2048 --tag gw5-test -- tests/rust-sandbox/run-tests.sh --lib -- services::update::release services::update::admin` | 17 passed |
| Unit suite | `tools/heavy.sh --min-mb 2048 --tag gw5-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | lib **424 passed / 17 ignored** (A2: 407 + 17), launch 16 passed / 1 ignored, `logs/guard-calls.log` empty |
| Helpers, policies, install-system.py, hint | `W11_TEST_TMP=$PWD/.rt/gw5 tests/update/test_helpers.sh` | **124 passed, 0 failed** |
| Installer | `tests/installer/run.sh` (REAL cases included, under heavy.sh) | **37 tests OK** (the byte-for-byte nl cases equal 3b898e6's output: the hint prints only on a terminal) |
| Package | `WIN11_BUILD_DIR=<12 placeholder executables> WIN11_PACKAGE_DIR=.rt/gw5/dist tools/heavy.sh --min-mb 1024 --tag gw5-package -- python3 -B scripts/package.py $PWD`, then `tar -tvzf` | rc 0; the tarball carries `host/data/` with the helpers 0755 and the policies 0644 (tarball removed afterwards) |
| After the last edit (`held` only in Settings hosts) | `… run-tests.sh --lib -- services::update` and `cargo check … --all-targets` | 82 passed / 9 ignored, guard log empty; warnings as above |
| Drift + ratchet | `python3 -B tools/i18n/drift_rust.py --paths host/src/services/update/{release,admin,paths}.rs --warnings`, then plain `--warnings` | **0 errors**; 2 / 3 warnings, all `classed technical` file names: `release.rs` 'gschemas.compiled' (new, GW5), `paths.rs` 'usr/share/glib-2.0/schemas' and `options.rs` 'gschemas.compiled' (GW4's, as before) |
| Catalogue | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` | **0 errors**, 68 warnings: 60 `[unused]` (the GW6/GW7/GW9 rows; GW5 now uses `err.promptFailed`, `.checkFailed`, `.ubuntuHold`, `.noAttachHelper`, `.noPassword`, `.wrongPassword`, `.notHeld`) + the 8 of before (glossary, space) |
| Pairs | `python3 -B tools/i18n/check_pairs.py --warnings` | 0 errors, 0 warnings (py-files 7, tables 12, pairs 165) |
| UI checker | `PATH=…node… python3 -B tools/i18n/check_ui.py --warnings`; `… --dict-diff tests/i18n/dict-nl-3b898e6.tsv --changes tests/i18n/BASELINE-CHANGES.ui --quiet` | 0 errors, 50 warnings (all glossary, none in the changed files); dict diff 270 added (the 11 GW5 keys among them), 0 not in BASELINE-CHANGES.ui |
| Mock rows | `python3 -B -m unittest tests/i18n/test_update_mock_rows.py` | in the worktree 1 failure until the merge (`BaselineAdditions`: the 11 keys are in the side file); with the side file merged into a copy (`.rt/gw5/merged/BASELINE-ADDITIONS.ui`) 4 ok + 1 skipped |
| node | `node --check --input-type=module < F` for update-release.js and mock/update.js | 2 × ok |
| Manifest | `python3 -B tools/ui-manifest.py --check`; `python3 scripts/verify-ui.py` | up to date; OK (181 files) |
| Dumps | `tools/heavy.sh --tag gw5-dump -- tests/i18n/run.sh --lang nl\|en --only settings.update,settings-update.H01,settings-update.H03,settings-update.H25 --out .rt/gw5/dump-<lang>.txt` | against the baselines only the Prompt row's lines are new in each block (title, 2 description lines, relNotOffered where not offered, the ComboBox's aria-label and value); nothing else changed in nl or en |
| Shots | `SUFFIX=-gw5 tools/heavy.sh --tag gw5-shots -- tests/update-ui/shoot.sh settings-update.H01`, `LANGS=en SIZE=800x1400 SUFFIX=-gw5-narrow …` | nl/en 1280 and en 800 (`.rt/gw5/shots/`): the row fits, no clipping; the narrow layout as the backup rows (after the flex-basis fix) |

test_helpers.sh in detail: the Prompt helper refuses 8 malformed calls (rc 2, `@@fail usage`, file unchanged), a call
as the user without `WIN11_HELPER_ETC` (rc 3 `notroot`) and a relative one (`etc`); on Ubuntu's file `lts` changes
exactly the Prompt line (`diff` = 1 line, 0644, the user's ownership, no temp left), `never`, then `normal` is
byte-identical again; the same text through `sh -c … win11-gnome-prompt set-prompt lts` and under `bash --posix`; six
rewrite rules (insert position, key case/colon/spaces, continuation + duplicate, no `[DEFAULT]`, no final newline,
blank lines); a missing file is created 0644; overrides named in order and left alone; refusals for a symlinked
file (target untouched), no directory, a directory in the file's place; a stale temp file and a temp-file symlink
(never followed). The attach helper refuses an argument (2) and a non-root caller (3) and execs exactly
`/usr/bin/screen -d -r ubuntu-release-upgrade-screen-window`; both helpers have `set -u`, the fixed PATH, umask 022,
ASCII only. The policies parse with `xml.etree` (ids, en and nl description/message, defaults, `exec.path`; attach
has no `_keep`). install-system.py: `outcome()` (126/127/markers) and `parse()` by import; the install script on a temp
root (markers, modes, contents, idempotent, nothing elsewhere; a wrong checksum installs nothing; an unknown file, a
relative stage, missing checksums, a non-root call without the seam refused); the remove script (exactly the four
paths and the empty directory, the polkit directory kept, a second run does nothing, a foreign file keeps the
directory); in bubblewrap with the pkexec stub under strace: `--dry-run` prints exactly `[<stub>, /bin/sh, -c,
INSTALL_SCRIPT, win11-gnome-system-install, <stage>, 4 checksums]` and the four `sha256  path` lines, runs no pkexec,
writes only inside its stage (14 calls, 0 outside) and removes it; `--remove --dry-run` / `--dry-run --remove` in en
print the remove command, no Dutch word, no write; 8 refused argument lists (rc 2, the usage line, no write); the
usage in en; root (a user namespace mapped to 0) refused with no write; a 0755 `XDG_RUNTIME_DIR` refused with no
write; pkexec never ran. The hint: missing → the install line, present → the remove line, nl and en.

## 4. Robin-only actions

* **R2** (after GW5 is reviewed and merged): `cd /home/robin/Documents/Codex/2026-11-01/chat/outputs/win11-gnome &&
  python3 scripts/install-system.py --dry-run && python3 scripts/install-system.py` (one password; GNOME's dialog shows
  `/bin/sh` because this step is the one that installs the nicer texts). Check: `ls -l /usr/libexec/win11-gnome/
  /usr/share/polkit-1/actions/io.github.win11shell.gnome.*` (two files 0755 root:root, two 0644 root:root) and
  `sha256sum host/data/* /usr/libexec/win11-gnome/* /usr/share/polkit-1/actions/io.github.win11shell.gnome.*`
  (pairs equal). Undo: `python3 scripts/install-system.py --remove`.
* **R4** (after R2 and a deploy of a build with GW5, R3/R9): `cp /etc/update-manager/release-upgrades
  ~/release-upgrades.before`; Settings › Update en herstel › Meer opties › Nieuwe Ubuntu-versies → "Alleen LTS-versies"
  → the password dialog says "Win11 Shell wants to set whether you get only LTS versions or every new version of
  Ubuntu." (en_US agent, D-G11) → `grep -n '^Prompt' /etc/update-manager/release-upgrades` shows `Prompt=lts`,
  `diff ~/release-upgrades.before /etc/update-manager/release-upgrades` shows only that line, `stat -c '%a %U:%G'
  /etc/update-manager/release-upgrades` is `644 root:root` → back to "Elke nieuwe versie" (or keep lts: decision 4 /
  R10) → `cmp ~/release-upgrades.before /etc/update-manager/release-upgrades` is silent (when the file ends with a
  newline, as Ubuntu's does; a file without one gets one: the helper writes whole lines). Optional before R2: the
  row shows the InfoBar "Ubuntu vraagt om je wachtwoord om ‘/bin/sh’ …" and the change shows the `/bin/sh` dialog.
  While the password dialog is open the page's request waits (review n8): note whether the page stays usable.

## 5. Deviations and decisions for the coordinator

1. **D1 new host row** `update.release.err.notHeld` ("Dit kan alleen zolang {product} nog niet klaar is voor de
   nieuwe Ubuntu-versie." / "This is only possible while {product} isn’t ready for the new Ubuntu version yet."):
   §4.3's "only while the verdict is hold/blocked" needs a refusal text; §3.7 has none. Listed in
   `BASELINE-CHANGES.rust-gw5`.
2. **D2 fallback argv**: `pkexec /bin/sh -c '<script>' win11-gnome-prompt set-prompt <v>` (one word more than §6.3's
   `win11-gnome-prompt <v>`): the constant script is the helper file itself (`include_str!`), so the fallback runs
   exactly the script that test_helpers.sh tests.
3. **D3 install-system.py hardening**: the root script also takes the four sha256 values and installs only from its
   own root-owned copy after checking them, so another process of the user cannot swap a staged file between staging
   and `install` (the stage is the user's directory). The dry run prints them as part of the command. Both root scripts
   honour `WIN11_SYSTEM_ROOT` only when not root (the tests); pkexec clears the environment anyway.
4. **D4 the hint lines print only on a terminal** (`sys.stdout.isatty()`). Unconditional, they would change the
   captured output that `tests/installer` compares byte for byte with 3b898e6 (every case in the sandbox lacks the
   system files) and, after R2, `test_uninstall_nl_identical_to_baseline` too (the real `/usr/libexec` is visible
   there). If the line should be unconditional, the installer tests (GE1c's files) need to accept it.
5. **D5 the click path**: `update.release.check` runs only as a write, i.e. only when the page calls it; no read runs a
   program. The page does not call it yet: `update.js`'s `checkFor` action (not GW5's file) should call
   `update.release.check` right after `update.check`, without waiting for it (owner: R3-UI or GW6). Until then the
   offer comes from Ubuntu's own copies (update-notifier's check-new-release-gtk, the motd timer).
6. **D6 snapshot fields** additive to §4.3: `name, upgradeBroken, source, currentLts, lastCheck, checkError`;
   `setPrompt` answers `changed`; `release.check` answers `{started:false, skipped:'never'|'offline'|'metered'|
   'recent'|'running'}` when it does not run. For the `api-methods.json` docs (WU-FIX) and GW8's as-built section.
7. **D7 `update.changed` after a release change** carries `next_rev()` without writing the snapshot (the pk thread
   owns it); the page reloads because the event's `rev` is not the snapshot's. Two events may name the same number;
   nothing reads `rev` as an identity.
8. **D8 Paths**: the `// GW5` entries sit in the struct and in `rooted()` (two places, as GW4's), outside the exec
   table; no new tool (`Nice` is done in the child with `libc::nice(10)`).
9. **D9 Prompt-row layout**: one inline `flex-basis: 0` on the row's own text block (§2.6) instead of CSS (not GW5's
   file); Dutch is new on this row, nothing else moves.
10. **D10 mock ids**: M01-M04 (+ M01b, M04b) live under the surface prefix `settings-update-prompt.`, because
    `test_update_mock_rows.py` pairs `settings-update.` with the `[HR]` ids only.
11. **Merge**: append `notes/claude/surfaces-additions-gw5.json` to `tests/i18n/surfaces.json` and
    `notes/claude/baseline-additions-gw5.txt` to `tests/i18n/BASELINE-ADDITIONS.ui`; then re-record the dumps with
    `--accept settings.update,settings-update.*` (every Update block gains the Prompt row; the new
    `settings-update-prompt.*` blocks need no accept) and run
    `SUFFIX= tools/heavy.sh --tag gw5-shots -- tests/update-ui/shoot.sh settings-update-prompt.M01,settings-update-prompt.M01b,settings-update-prompt.M02,settings-update-prompt.M03,settings-update-prompt.M04,settings-update-prompt.M04b`
    (and `LANGS=en SIZE=800x1400 SUFFIX=-narrow` for M02/M03/M04): the M states other than M01 could not be rendered
    in the worktree without editing surfaces.json. `tests/i18n/MOCK-CHANGES.md` (not GW5's) may want a row for the
    Prompt scenarios and the live setPrompt. `tests/update/test_helpers.sh` is not under heavy.sh (no cargo/Chrome);
    it needs `bwrap` and `strace`.
12. **For GW6**: `release::detect` gives the offer (`codename`, `upgradeBroken` for the `ubuntuHold` state);
    `compat::cached(shared, codename)` is what `ubuntuHold {on:true}` checks; call `release::restore_hold(shared)` when
    the verdict becomes ok/warn or the codename changes; `relNotOffered` already exists in update-release.js's
    dictionary. **For GW7**: `admin::attach_argv(shared, lang)` → `[pkexec, attach helper]` or
    `update.release.err.noAttachHelper`; `admin::wait(child, timeout)` reads markers.
13. **Not changed, for Robin (D-G1b class)**: none found in GW5's area.

## 6. Fix round (2026-11-06, after `notes/claude/wu/gw5-review.md`)

Start state checked: `git status --short` and `git diff --stat` were exactly the review's view (10 modified files,
1783+/52−, the same untracked owned files), HEAD 18c8ffbe. Nothing committed. As before: no pkexec, check-new-release,
apt, gsettings/dconf, bus or network on the real system; the helpers ran only as Robin on temp etc dirs, the root
scripts on temp roots, `install-system.py` only with `--dry-run`/refused arguments in bubblewrap, or in-process with
`subprocess.run` and `shutil.which` replaced (nothing executed); every heavy command through `tools/heavy.sh`
(`gw5-fix-*` tags). Read-only, for m1: `/usr/lib/python3/dist-packages/UpdateManager/Core/MetaRelease.py` (`_parse_cfg`,
`parse`, `_buildMetaReleaseFile`) and `/usr/lib/python3.14/configparser.py` (`_read_inner`). Scratch: `.rt/gw5/fix/`
(logs, the corpus generator `gen_corpus.py`, a merged copy `root/` with the two side files appended, its dumps in
`root/.rt/` and shots in `root/tests/update-ui/shots/`).

### 6.1 Findings

| Id | Verdict | What changed |
|---|---|---|
| M1 | **not fixed here** (outside GW5's files) | `update.js` is not GW5's. The host side is unchanged and ready. Proposed patch for its owner (R3-UI or GW6), to land only after m6's seams: in `ACTIONS`, `checkFor: { label: () => t('checkFor'), run: (b) => { void call('update.release.check', {}); return write('update.check', {}, b); } },` (`call` is common.js's, already imported, never throws; the answer, `{started}` / `{started:false, skipped}` / `{dryRun:true}`, is ignored on purpose: the offer arrives with the next `update.changed`). |
| m1 | fixed | `config_prompt` is now a port of Python 3.14's `configparser._read_inner` (as `_parse_cfg` uses it: one parser per file, any `configparser.Error` ignores the file): `\r\n`/`\r`/`\n`, Python's whitespace set, comments/blank lines inside values, indentation continuations, `[` + name + last `]` headers, first `=`/`:`, and a duplicate key, duplicate non-DEFAULT section, key before any section, junk line or empty key → the file is ignored. The helper's awk follows the same reader in three modes (`rewrite`, `check`, `probe`); the rewritten temp file must pass `check` (Ubuntu reads exactly `Prompt=<v>`) before chown/chmod/sync/mv, else `@@fail parse` and the old file stays. CRLF is kept per line, the key's indentation is kept, an inserted key takes the next header's indentation (so an indented header stays a header). A lone CR or a Unicode space outside a comment line always fails `check` (awk cannot judge them as Python does). `set_prompt` now requires that the main file reads as the wanted value afterwards (or that a `.d` file decides), else `promptFailed`. Corpus `tests/update/fixtures/release/cfg/` (39 files + `expected.tsv`: configparser's verdict, the helper's outcome, the override expectation), checked by test_helpers.sh against python3 at test time (117 helper runs, each @@ok verified by configparser, each refusal byte-identical) and by the unit test `config_prompt_reads_each_file_as_configparser_does`. The reviewer's cases: CRLF → @@ok, Ubuntu reads lts; `[DEFAULT] ; main` → @@ok, no second DEFAULT; a key before any section → `@@fail parse`, file kept; the indented `Prompt=never` after another key reads as none (continuation), as configparser does. |
| m2 | fixed | The stamp compares year.month only and never offers the current codename; fixture `stamp/current-point-en` ("26.04.1 LTS" on 26.04 → no offer). |
| m3 | fixed | `outcome()` returns the `@@ok` paths for every outcome; after pkexec `main()` prints `Installed:`/`Removed:` for each, then `failedPartly` "That didn’t work ({code}). What was already done is listed above; try again." / "Het is niet gelukt ({code}). Hierboven staat wat al wel is gebeurd; probeer het opnieuw.", or, with nothing done, `failed` "That didn’t work ({code}). Try again." / "Het is niet gelukt ({code}). Probeer het opnieuw." Tested in-process (nl, en, install and remove, halfway, nothing, success). |
| m4 | fixed | nl `doneInstall` "Klaar. Instellingen gebruikt …"; en `intro` "Win11 installs four system files, so that Ubuntu’s password prompt says what it’s for when you use ‘New Ubuntu versions’ and ‘Show the upgrade window’:", nl `intro` with the two names quoted; en `dryRun` "… and you won’t be asked for a password."; the names quoted in `doneRemove` and in install.py's hint too. Optional part taken: `promptOverride` en "This setting is managed by {file}." (24H2's "managed by"; nl unchanged). |
| m5 | fixed (design reading) | `promptShInfo` now stands under the row whenever `release.helper.prompt === false` and the row can change the file (no `.d` override, no running upgrader), and stays while a change waits: design §3.6 "helper missing: InfoBar". Consequence: every Update surface of the mock (no root helper, today's state) gains that InfoBar line; M04 differs from M01 by the pending change (ComboBox disabled). |
| m6 | **not fixed here** (outside GW5's files), latent | The fix is harness.py's (R3-NATIVE, D-G44: every harness Settings run gets `WIN11_UPDATE_TOOL_DIR`/`WIN11_UPDATE_FIXTURE_ROOT`) plus a harness self-test that no execve of `/usr/bin/pkexec` or `check-new-release` happens; M1 lands only after that. Production `Tools` must keep absolute paths, so GW5's code has nothing to change. Today: nothing calls `release.check`, and no harness scenario changes the Prompt row. |
| n1, n2 | coordinator | as the review says (n2: the D6 fields and `MOCK-CHANGES.md`). |
| n3 | fixed | After a failed run (`release.json` `error` set) a click may check again after 1 h (`CHECK_RETRY_MS`); a good run still counts for 24 h. Tested. |
| n4 | fixed | `parse_meta_release` gives None without any stanza (MetaRelease deletes an empty copy), so an empty newer copy no longer hides an older one. Tested. |
| n5 | fixed | The row is disabled (and without the InfoBar) while `phase === 'releaseRunning'` or `release.state === 'running'` (H25: shot checked). |
| n6 | kept, reason | `shutil.which("pkexec")` is what the stub test needs, the dry run prints the resolved path, and a test variable would be the same seam with a typo risk towards the real binary. |
| n7 | kept, reason | Running install.py/uninstall.py on a pty would be a real install outside tests/installer's sandbox; the source grep plus `system_hint()` by import stay. |
| n8 | R4 | one line added to R4 (§4). |
| n9 | fixed | the `/etc/passwd` assertion runs only when `/etc` and `/etc/passwd` are uid 0; in tests/update/run.sh's layer all 18 GW5 tests now pass. |
| n10 | fixed | sentence in R4 (§4). |
| n11 | GW8 | M02's mock keeps `released:null`; a real interim release with lts may also show `relNotOffered`. |

### 6.2 Changed in this round

`host/src/services/update/release.rs` (`py_space`, `py_lines`, `config_prompt`, the stamp, `parse_meta_release`,
`CHECK_RETRY_MS`, `set_prompt`'s after-check, module doc; tests: 1 new, 5 extended), `host/src/services/update/admin.rs`
(n9 only), `host/data/release-upgrades-helper` (the configparser-faithful awk with `check`/`probe`, `@@fail parse`;
still `/bin/sh`, `set -u`, fixed PATH, umask 022, ASCII only), `scripts/install-system.py` (m3, m4),
`scripts/install.py` (the hint's quoted names), `vendor/ui/js/apps/settings/pages/update-release.js` (m5, n5,
`promptOverride` en), `vendor/ui/js/mock/update.js` (header comment only), `vendor/ui-sha256.json` (regenerated),
`tests/update/test_helpers.sh` (corpus, CRLF/header/ignored-file cases, the in-process m3 test, updated texts, the
indentation rule), new `tests/update/fixtures/release/cfg/*` (39 `.cfg` + `expected.tsv`) and
`tests/update/fixtures/release/stamp/current-point-en`, `notes/claude/{surfaces-additions-gw5.json,
baseline-additions-gw5.txt}` (comments only), this note (§2, §4 and this section). Not touched: `paths.rs` in this
round, `update.tsv` (no new host row), every file outside GW5's list.

### 6.3 Verification (all from `.wt/gw5`)

| Check | Command | Result |
|---|---|---|
| GW5 unit tests | `tools/heavy.sh --min-mb 2048 --tag gw5-fix-test -- tests/rust-sandbox/run-tests.sh --lib -- services::update::release services::update::admin` | **18 passed** (17 + the corpus test) |
| cargo check | `tools/heavy.sh --tag gw5-fix-check -- cargo check --offline --locked -j2 --all-targets` | rc 0; lib 28, lib test 26 (25 dup), win11-launch 2 (+1 test) warnings: as before, none in `services/update/` |
| Unit suite | `tools/heavy.sh --min-mb 2048 --tag gw5-fix-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | rc 0; lib **425 passed / 17 ignored**, win11-launch 16 / 1 ignored, `guard-calls.log` 0 bytes |
| GW5 tests in the strict layer | `tools/heavy.sh --min-mb 1024 --tag gw5-fix-layer -- tests/update/run.sh exec --no-strace -- sh -c 'strace -f -e trace=execve,connect … BIN services::update::release services::update::admin'` | **18/18** (n9). execve: only the fixture's stubs (check-new-release 6, pkexec 15, release-upgrades-helper 6, sh 2), `/bin/sh` and the helper's awk/id/chown/chmod/sync/mv/rm on temp etc; no `/usr/bin/pkexec`, check-new-release, `/usr/libexec/win11-gnome`, dconf, gsettings or apt. connect: the layer's own bus and nss/nscd only. Layer stub log 0 calls, fake CallCount 0 |
| Bus tests (regression) | `tools/heavy.sh --min-mb 1024 --tag gw5-fix-bus -- tests/update/run.sh bus-tests -- target/debug/deps/win11_host-12500e7eb53cf9c1` | 8 passed, 1 failed: `progress_under_the_flood_timeline` "more than 4 page events in one second" (5; load average 3.9 while another agent's bus tests ran). Alone again (`--only progress_under_the_flood_timeline`, `gw5-fix-bus2`): **1 passed** (at most 4 per second, nl and en). GW5 touches nothing on that path (the pk thread's throttle); a load-sensitive GW2 timing check, for the coordinator |
| Helpers, policies, corpus, install-system.py | `W11_TEST_TMP=$PWD/.rt/gw5/fix tests/update/test_helpers.sh` | **131 passed, 0 failed** (124 + corpus, CRLF, `[DEFAULT] ; main`, ignored file + no temp file, the m3 test + stages removed); dry run writes inside=14 outside=0; pkexec stub never ran |
| Installer | `tests/installer/run.sh` (REAL cases, its own heavy.sh) | **37 tests OK** |
| Package | 12 placeholder executables, `TMPDIR`/`WIN11_BUILD_DIR`/`WIN11_PACKAGE_DIR` under `.rt/gw5/fix/pkg`, `tools/heavy.sh --min-mb 1024 --tag gw5-fix-package -- python3 -B scripts/package.py $PWD`, `tar -tvzf` | rc 0; `host/data/` with both helpers 0755 and both policies 0644 (tarball removed) |
| drift / ratchet | `python3 -B tools/i18n/drift_rust.py --paths host/src/services/update/{release,admin,paths}.rs --warnings`; plain `--warnings` | **0 errors**; 2 / 3 warnings (the technical file names of before) |
| catalogue | `python3 -B tools/i18n/check_rust.py --warnings` | **0 errors**, 68 warnings (as before) |
| pairs | `python3 -B tools/i18n/check_pairs.py --warnings` | **0 errors, 0 warnings** (pairs 166: install-system.py's new `failedPartly`) |
| UI checker | `check_ui.py --warnings`; `--dict-diff tests/i18n/dict-nl-3b898e6.tsv --changes tests/i18n/BASELINE-CHANGES.ui --quiet` | 0 errors, 50 glossary warnings (none in GW5's files); 270 added, 0 not in BASELINE-CHANGES.ui, 0 stale |
| node, manifest | `node --check --input-type=module` (update-release.js, mock/update.js); `tools/ui-manifest.py` then `--check`; `scripts/verify-ui.py` | 2 × ok; regenerated, up to date (181 files, `ui-patches.json` unchanged); OK |
| Mock rows | `python3 -B -m unittest tests/i18n/test_update_mock_rows.py` in W / in `.rt/gw5/fix/root` (side files appended) | W: 1 failure (side file not merged, as before), 1 skipped; merged: **4 ok, 1 skipped** |
| Dumps | merged copy: `tools/heavy.sh --tag gw5-fix-dump -- .rt/gw5/fix/root/tests/i18n/run.sh --lang nl\|en --only settings.update,settings-update.H01,…H03,…H25,settings-update-prompt.{M01,M01b,M02,M03,M04,M04b} --out .rt/gw5/fix/root/.rt/dump-<lang>.txt` | against `tests/i18n/baseline-{nl,en}.txt`: settings.update, H01, H03 **−0 +7** (the row's 6 lines + the promptShInfo InfoBar), H25 **−0 +5** (row disabled, no InfoBar); no existing line changed in nl or en; 6 new prompt blocks; no Dutch word in the en prompt blocks; M03 en "This setting is managed by …" |
| Shots | merged copy `shoot.sh`: H01, H25, M01–M04b nl/en 1280 (16); H01, M02, M03, M04, M04b en 800 (5); M01, M04 nl dark (2) | 23 of 23, rc 0; viewed H01/M01/M02/M03/M04/M04b/H25 and the narrow and dark ones: the InfoBar sits under the card like the error InfoBar did (one line at 1280, three at 800), nothing clipped or overflowing, H25 and M03 greyed without the InfoBar, the card look unchanged |

### 6.4 What is left, decisions for the coordinator

1. **M1** (owner R3-UI or GW6, after m6): the one-line `checkFor` patch in §6.1.
2. **m6** (R3-NATIVE / harness): the update seams for every harness Settings run and a no-execve self-test for
   `/usr/bin/pkexec` and `check-new-release`; GW8 runs the M states with the seams.
3. Merge as §5 item 11, with one change: every Update block gains **7** lines (the InfoBar included; H25/H25b gain 5),
   so `--accept settings.update,settings-update.*` after appending the side files.
4. Deviations added in this round, for the as-built notes: D12 the helper's `@@fail parse` (new marker; nothing in the
   design names it) and its configparser-faithful reader; D13 `promptShInfo` shown whenever the helper is missing
   (design §3.6 literally), so every mock Update surface shows it; D14 `promptOverride` en "managed by" instead of
   the design's "controlled by"; D15 the 1-hour retry after a failed release check (the design's "once per 24 h"
   stays for good runs); D16 install-system.py's two failure texts (`failed`, `failedPartly`) replace the one that
   pointed at output it never printed.
5. Observation: `progress_under_the_flood_timeline` (GW2b) failed once under load (5 page events in a second, limit
   4) and passed alone; worth a look if it recurs.
