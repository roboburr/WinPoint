# GW7: the hand-over to Ubuntu's upgrader and the post-upgrade check (2026-11-06)

Step GW7 of `notes/claude/windows-update-gnome.md` §10 (design §6.4, §6.5, §6.6, §3.5, §3.7, §4.3, §4.5, §4.9, §4.11,
§9, §11 R5/R9), in the worktree `.wt/gw7` (branch `r4/gw7`, from 4e0323eb). Start state checked: `git status --short`
empty, HEAD 4e0323eb5b0bd0aceb86860459a82b51ae9057b8. Nothing committed, no git state changed. No real system bus, live
session bus, PackageKit, apt, loginctl, pkexec, do-release-upgrade, xdg-terminal-exec, systemd-run, gsettings, dconf or
network was used; no win11-* binary ran outside `tests/rust-sandbox/run-tests.sh` and `tests/update/run.sh exec`. Every
program of the code under test was a stub of a marked tool directory; every user manager, GNOME Shell and notification
server was a Python + Gio fake on a private session bus inside run.sh's bwrap layer. Local read-only/fixture tools used
outside a sandbox: `readelf -d` (on build snapshots), `glib-compile-schemas` (writing only
`tests/update/fixtures/upgrade/schemas/`), `rustfmt --edition 2024` (on my two new files only). Heavy commands through
`tools/heavy.sh` (`gw7-*` tags). Scratch files in `.rt/gw7/`.

## 1. Files

| File | What |
|---|---|
| `host/src/services/update/upgrade.rs` | was GW2's stub; now `update.release.start` (preconditions, `upgrade.json`, linger, terminal), the `WRAPPER` constant, `update.release.attach`, the owner's watch (`watch`, `shown`, `clear_shown`), the card overlay (`overlay`, pure `card_state`); 10 unit tests + 1 ignored layer test |
| `host/src/services/update.rs` | only `// GW7` blocks (11 hunks): `anyway` in the method table and allowlist; the field `upgrade_watch`; `watch_upgrade()` (owner only, once on every write and every `update.status`, then a 5 s `glib::timeout_add_seconds` while the hand-over is unresolved; never armed under `cfg(test)`); a new owner picks a hand-over up; `update.check` clears a shown outcome; `upgrade::shown` after an owner's status; in `compose` the verdict is kept before GW2's `running` line and `upgrade::overlay` runs after it |
| `host/src/services/update/model.rs` | only `upgrade_outcome`'s region: the three `#[allow(dead_code)]` lines went (now used); logic unchanged |
| `host/src/files/actions.rs` | `user_environment` is `pub(crate)` (one line) |
| `host/src/post_upgrade.rs` | new: `win11-launch --post-upgrade-check` (§6.6), its own tool table and seam check, 8 unit tests |
| `host/src/bin/launch.rs` | the flag (exactly one argument) and the `#[path]` includes of `post_upgrade.rs` and `process_scope.rs` (`#[allow(dead_code)]`) |
| `scripts/install.py` | `T["postUpgradeName"]`, `AUTOSTART`, `autostart_entry()`; writes `$XDG_CONFIG_HOME/autostart/io.github.win11shell.PostUpgradeCheck.desktop` after the app entries; the path joins the first-install conflict list |
| `scripts/uninstall.py` | removes that file (both modes); every other autostart entry stays |
| `tests/installer/sandbox.py` | `AUTOSTART*` and `Case.tree(autostart=False)` (§3 D13) |
| `tests/installer/test_installer.py` | new class `Autostart` (4 cases); no existing case changed |
| `tests/update/fixtures/upgrade/` | wrapper status files (rc 0, 1, 126, 127), five `apt-cache policy`/`showpkg` outputs, a two-key `org.gnome.shell` schema + `gschemas.compiled` |
| `tests/update/test_post_upgrade.py` | new layer test (fakes: org.gnome.Shell, org.gnome.Shell.Extensions, org.freedesktop.Notifications, org.freedesktop.systemd1), 17 scenarios |
| `notes/claude/api-docs-gw7.md` | doc strings for `api-methods.json` (WU-FIX's file) |

Not touched: `paths.rs`, `release.rs`, `guard.rs`, `admin.rs`, `host/i18n/update.tsv` (every text GW7 needs was already
a row; so no `BASELINE-CHANGES.rust-gw7` and no `mock/update.js` host-rows line), `vendor/ui/**` (no manifest
regeneration), `api-methods.json`, other steps' trees.

## 2. What was built

### 2.1 `update.release.start {anyway?:bool}` (§6.4 steps 1–4)

* A start of ours that is still on its way or running answers `{started:true, already:true}`: no second terminal.
* Preconditions in Ubuntu's order, each a §3.7 key: `release.offered` false → `notOffered`; **updatesPending**: any
  snapshot package with `pup` null (no Phased-Update-Percentage; held ones count) → `updates_pending` with `data
  {count, held}` (`held` = the names when only held packages block); **rebootFirst**: `/run/reboot-required` with
  `libc6`, `linux-base` or `linux-image-*` in `.pkgs` (gnome-shell alone does not count); **busy**: our `job.json`
  unfinished in this boot with a live owner, the snapshot's `foreign`, an own Software job, or a held dpkg/apt lock
  (`guard::apt_lock_held`, `/proc/locks` against the four lock inodes); **hold**: the verdict must be `ok`/`warn`;
  `hold` passes only with `anyway:true` and the release's `allowAnyway:true`; `ubuntuHold` → `notOffered`; every other
  verdict (`none`, `checking`, `unknown`, `blocked`) → `hold`. While Ubuntu's upgrader runs, GW2's guard answers first
  (`update.guard.upgrade`).
* Then: an older record and its status files go; `$XDG_RUNTIME_DIR/win11-gnome/` (0700, ours) with a fresh
  `upgrade-status.json`; `upgrade.json` `{jobId, startedAt, bootId, version, codename, anyway, lingerByUs, seenRunning,
  seenAt, outcome, shownAt, card}`; linger (§2.3); the terminal; `update.changed`. A failed terminal turns our linger
  off again and removes `upgrade.json` (`noTerminal`).
* The terminal: `[xdg-terminal-exec, --title=<update.release.terminalTitle>, --, <sh>, -c, WRAPPER,
  win11-gnome-upgrade, <status-file>, fail, <pkexec>, <do-release-upgrade>]`, all four programs from `Tools`, no
  `-f/-d/-p/--allow-third-party`, no sudo. Environment `files::actions::user_environment(true)` minus `WIN11_UPDATE_*`,
  plus `LANGUAGE` (`nl:en` / `en`), `WIN11_PRESS_ENTER` (`update.release.pressEnter`), `PWD=$HOME` and, in production,
  `DESKTOP_STARTUP_ID`/`XDG_ACTIVATION_TOKEN` from `launch::activation_token` for the default terminal's desktop id;
  in test mode instead the tool table's `PATH` and dead bus addresses. cwd `$HOME`. Started by
  `crate::process_scope::spawn(command, "release-upgrade", Some(env))`: Role::App → `Slice=app.slice`.
* `WRAPPER` is the design's text byte for byte (a raw string; `post_upgrade.rs` holds the same text, a lib test checks
  it).

### 2.2 Watching, the outcomes, the card (§6.4 step 5, §3.5)

* `watch()` (owner, serialized by one mutex with start/shown/clear): a record of another boot → linger off, removed.
  The `/proc` scan (`guard::upgrader_running`) sets `seenRunning`/`seenAt`; `model::upgrade_outcome(running, seen,
  status rc, since start, /run/reboot-required)` decides: Starting/Running → card `running`, or `waiting` when
  `main.log` is ≥ 10 min older than now, counted from `max(mtime, seenAt)`; Restart → recorded, card hidden (`none`),
  linger kept for the reboot, record kept for the post-upgrade check; StartFailed(rc)/NoWindow → recorded with the
  §3.7 key, linger off; Ended → recorded with the rc, linger off; Offer → linger off, files removed at once. A seen
  upgrade appends `{jobId, role:'release', startedAt, finishedAt, outcome}` to `jobs.jsonl` (GW4 decision 6). A card
  change emits `update.changed`. The timer stops once an outcome is recorded.
* "Once the card has shown it": the owner's `update.status` that returns startFailed/ended sets `shownAt`; the next
  `update.check` (or a new start, or another boot) then removes `upgrade.json` and the status files.
* `overlay()` (every host, files only): `state` = running/waiting (any upgrader, also one we did not start) >
  our startFailed/ended (and `none` for Restart) > `updatesFirst` > `rebootFirst` (both only while offered and the
  verdict is ok/warn) > the verdict. Fields in §5.

### 2.3 Linger (§6.4 step 3)

`loginctl show-user <uid> -p Linger --value`; `no` → `loginctl enable-linger <uid>`, success → the stamp
`<config>/update-linger-by-us` (`{uid, since}`, 0600, atomic) and `lingerByUs:true`. `yes` → ours only when our stamp is
there (an earlier start of ours). Any other answer → no linger, the start goes on. Off: only while the stamp exists and
no upgrader runs; `loginctl disable-linger <uid>`, the stamp goes on rc 0.

### 2.4 `update.release.attach` (§6.4 step 6)

Only while Ubuntu's screen session (`screen`/`SCREEN … -S ubuntu-release-upgrade-screen-window`) is in `/proc`, else
`update.release.err.notRunning`; `admin::attach_argv` (`noAttachHelper` with the manual command when the helper is not
trusted); the same terminal path, `$0 win11-gnome-attach`, hold `fail`, its own `attach-status.json`.

### 2.5 The guard (§6.4 step 7, §4.9)

Checked, nothing added: `upgrade_guarded` refuses check, install, confirm(accept), repair, release.check/compat/
setPrompt/start while the upgrader runs; `guard::power_blocked` and `guard::restart_all_blocked` refuse power actions
and `shell.restartAll`. §6.4 step 7's ".release.* except attach" would also cover `release.ubuntuHold`, but D-G40
chose §3.7's list and two GW2 tests pin exactly that list (`every_write_is_refused_while_the_release_upgrader_runs`
asserts it; the bus test `upgrade_running_refuses_every_write` asserts ubuntuHold is *not* refused): decision 1 in §6.

### 2.6 `win11-launch --post-upgrade-check` (§6.6)

* Seams: no `WIN11_UPDATE_*` → production; any → only the three known names, euid ≠ 0, a marked private
  `WIN11_UPDATE_FIXTURE_ROOT` (required; every system path re-rooted) and optionally `WIN11_UPDATE_TOOL_DIR` (stubs
  only); a failure prints one stderr line and does nothing.
* While Ubuntu's upgrader runs: nothing at all (the next login checks).
* `os-release` + `ShellVersion` (`Properties.Get`, NO_AUTO_START, retried every 0.5 s for 30 s; 2 s in test mode),
  else `dpkg-query -W -f=${Version} gnome-shell` → the major. No record → write it (+ linger cleanup) and stop.
  Unchanged (versionId, codename, a known major) → only the linger cleanup when the stamp exists.
* Changed: `org.gnome.shell` (default source/backend; test mode: the fixture's compiled schema + a keyfile)
  `disable-user-extensions` true → extsOff [extsOn, dockOn]; else `GetExtensionInfo` `state` as a number (d, or an
  integer type), rounded; missing/non-numeric → ERROR. 1 → upgraded; 4, or 2 with our uuid in `enabled-extensions` →
  extOff [dockOn, later] (with an unknown major: extError instead); 3 → extError [dockOn, later]; others nothing.
  Missing sonames (12-entry table, the NEEDED set of the 12 binaries + libarchive; a test reads
  `tests/build/base-debug`'s ELF files) → missing [installParts, later], body `{parts}` = labels joined with ", ".
* Linger cleanup, then the notifications (app `Win11 Shell`, icon `system-software-update`, hint
  `desktop-entry=io.github.win11shell.Settings`), subscribed to ActivationToken/ActionInvoked/NotificationClosed of
  `org.freedesktop.Notifications` before the first Notify; the main loop waits until each notification with actions
  was answered or closed, at most 10 min (8 s in test mode). Only an ActionInvoked whose key that notification offered
  acts: dockOn → `EnableExtension("ubuntu-dock@ubuntu.com")`; extsOn → `disable-user-extensions=false` (+ sync in
  production); installParts → the names (`apt-cache policy <old>` candidate → old; else exactly one "Reverse Provides:"
  package of `apt-cache showpkg`; else old) and `[xdg-terminal-exec, --, <sh>, -c, WRAPPER, win11-gnome-parts,
  <runtime>/parts-status.json, always, <pkexec>, <apt-get>, install, names…]` through `process_scope::spawn` (Role::App),
  with the notification's activation token.
* Then `last-os-release.json` `{versionId, codename, gnomeMajor, extensionVersion}` (0600, atomic) and the removal of
  `compat-*.json`, `release.json`, `upgrade.json`, `upgrade-status.json`, `attach-status.json`.
* Texts: `crate::i18n` of the bin (the embedded `host/i18n/update.tsv`), language from `settings.json` (C1).

### 2.7 The autostart entry (§6.6)

```
[Desktop Entry]
Type=Application
Name=Win11-controle na een Ubuntu-upgrade        (en: Win11 check after an Ubuntu upgrade)
Exec=/usr/bin/env -- "<prefix>/bin/win11-launch" --post-upgrade-check
OnlyShowIn=GNOME;
NoDisplay=true
```

Name in Win11's language at install time (the same `language` as the app entries, D-G4); `Exec` quoted by
`desktop_exec`; no `X-GNOME-Autostart-Delay`. Written on every install and update; removed by `uninstall.py`
(also `--files-only`).

## 3. Deviations and readings

1. **D1 `update.release.start {anyway?:bool}`** (design `{}`): the hold-with-warning of §12 C4 needs the page to say
   the warning was accepted; the host checks it against the release's `allowAnyway`.
2. **D2 Starting shows `running`**: between the start and the first sighting (the pkexec dialog, ≤ 120 s) §3.5 has no
   state; the card shows `running` (attach then says notRunning, and `attachable` is false).
3. **D3 attach needs Ubuntu's screen session**, not just an upgrader: the helper can only resume that session (the GTK
   frontend has none). Also keeps GW2's upgrader-running tests tool-free. The card gets `attachable`.
4. **D4 attach's own status file** (`attach-status.json`) and `$0` `win11-gnome-attach`: sharing `upgrade-status.json`
   would overwrite the upgrade's rc.
5. **D5 loginctl with the explicit uid** for show-user, enable-linger and disable-linger (the design's argument-less
   enable-linger acts on the caller too; logind asks `set-self-linger` for uid == caller either way).
6. **D6 the outcome record lives until shown + the next check** (or a new start, or another boot); the offer outcome
   goes at once; the reboot outcome stays for the post-upgrade check. `shownAt` is set only by the owner's status.
7. **D7 stricter start**: only ok/warn (or hold + allowAnyway + anyway) start; `ubuntuHold` → notOffered.
8. **D8 the guard is unchanged** (§2.5, decision 1).
9. **D9 test mode asks for no activation token** (no extension call); the terminal env gets the tool table's PATH and
   dead buses; `WIN11_UPDATE_*` never reach the terminal.
10. **D10 no `paths.rs` block**: the runtime dir is `glib::user_runtime_dir()/win11-gnome`, in test mode
    `<root>/home/.run/win11-gnome`, computed in `upgrade.rs` (same rule in `post_upgrade.rs`).
11. **D11 the post-upgrade texts use the bin's `#[path]` `i18n.rs`** (already in win11-launch, the same embedded rows)
    instead of a second 20-line lookup.
12. **D12 post-upgrade order**: nothing while an upgrader runs; linger cleanup before the ≤ 10 min wait (a logout during
    the wait cannot leave linger on); the record and the cache cleanup after it (so an unanswered notification comes
    back at the next login). Test mode waits 2 s for ShellVersion and 8 s for an answer. The parts terminal has no
    `--title` (design §6.6's argv).
13. **D13 installer tests**: `Case.tree()` leaves out the autostart entry and an otherwise empty `config/autostart`
    unless `autostart=True`, because the byte-for-byte cases compare with 3b898e6's installer, which never wrote it;
    the new `Autostart` cases check exactly that these two paths are the only additions (nl first install against
    3b898e6), the text in nl/en, the Exec round trip with a nasty prefix, the uninstall (also next to a foreign
    autostart entry; 3b898e6's uninstall leaves it) and the first-install conflict. No existing test method changed.
14. **D14 `process_scope`'s 4 tests now also run in win11-launch's test binary** (the `#[path]` include): launch
    28 passed / 2 ignored (was 16 / 1; +8 post_upgrade, +4 process_scope, +1 ignored).
15. **D15 a `jobs.jsonl` line `role:'release'`** for a hand-over whose upgrader was seen (GW4 decision 6).
16. **D16** the 5 s timer runs the bookkeeping on the GTK main thread (a `/proc` scan, a few ms; `loginctl` once at an
    outcome).

## 4. Verification (from `.wt/gw7`)

| Check | Command | Result |
|---|---|---|
| cargo check | `tools/heavy.sh --tag gw7-check -- cargo check --offline --locked -j2 --all-targets --message-format=short` | rc 0; lib 28, lib test 26 (= baseline), win11-launch 1 (baseline 2: `configure_portal_child_environment` is now used), launch test 1; none in GW7's files |
| Unit suite | `tools/heavy.sh --min-mb 2048 --tag gw7-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | exit 0; lib **439 passed / 18 ignored** (429 + 10 / 17 + 1), win11-launch **28 / 2**; `logs/guard-calls.log` **0 bytes**; tmp entries 0 |
| GW7 lib tests | (in the suite) `services::update::upgrade::tests::*` | 10 passed: the exact argv/env in nl and en + `already`; the preconditions in order (every refusal with its text, no program ran), hold + allowAnyway + anyway starts; linger rules; the WRAPPER run by `/bin/sh` (rc 0 no hold; 1/126/127 held until Enter, exit = rc; `always` holds; `{rc,endedAt}`); status fixtures → startFailed 126/127/1, noWindow, ended ×2, offer, restart ×3, another boot; running/waiting from fixture procfs + `main.log` mtimes, attachable; the card precedence; attach; the service routing and the undo of a failed terminal; the shared WRAPPER text |
| post_upgrade tests | (in the suite) `post_upgrade::tests::*` | 8 passed: states 1/2/3/4 as doubles (13 cases incl. 3.9 → 4, NaN, 6, 99, disable-user true), integer variants; os-release/major/record change; apt-cache fixtures and `resolve_name` through a stub (candidate → old, one provider → it, two → old, unknown → old); soname table vs `tests/build/base-debug` NEEDED; the seams fail closed (7 cases); the upgrader rule; nl/en texts; the exec-path grep (one table, one command builder) |
| Layer test | `tools/heavy.sh --min-mb 1024 --tag gw7-layer -- /usr/bin/python3 -I -B tests/update/test_post_upgrade.py run --launch target/debug/win11-launch --lib target/debug/deps/win11_host-12500e7eb53cf9c1` | rc 0, **17/17 scenarios, 103 checks**; run.sh: stub log **0 calls**, fake PackageKit CallCount 0, **strace OK** (797 connect/send calls in 111 processes; successful connects only to `/tmp/w11-update/dbus-*`, datagrams only to the layer's own tmpfs) |
| Bus tests | `tools/heavy.sh --min-mb 1024 --tag gw7-bus -- tests/update/run.sh bus-tests -- target/debug/deps/win11_host-12500e7eb53cf9c1` | **9 passed, 0 failed** (the list equals bus-tests.json) |
| Installer | `tests/installer/run.sh` (REAL cases included; its own heavy.sh) | **Ran 41 tests, OK** (37 + 4 Autostart), 260 s |
| NEEDED | `readelf -d target/debug/win11-launch \| grep NEEDED` vs `tests/build/base-debug/win11-launch` | identical: libgobject-2.0, libgio-2.0, libglib-2.0, libgcc_s, libc, ld-linux-x86-64 |
| rustfmt | `tools/rustfmt-check.sh` | 143 files, 112 clean, 31 listed, **0 failing** (upgrade.rs and post_upgrade.rs clean) |
| drift / ratchet | `python3 -B tools/i18n/drift_rust.py --warnings` | **0 errors**, 5 warnings (2 new: `post_upgrade.rs` 'usr/share/glib-2.0/schemas', 'gschemas.compiled', classed technical, as GW4/GW5's) |
| catalogue | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` | **0 errors**, 64 warnings; no `update.release.*`/`update.post.*` row unused any more (left: GW6's compat/part/feature rows) |
| check.py | `python3 -B tools/i18n/check.py` | **0 errors** (64 warnings, all rust) |
| pairs | `python3 -B tools/i18n/check_pairs.py --warnings` | 0 errors, 0 warnings (pairs 167: + `postUpgradeName`) |
| mock rows | `python3 -B -m unittest tests/i18n/test_update_mock_rows.py` | OK (4 + 1 skipped); no new row, so no host-rows line |
| sites | `python3 -B tools/i18n/extract_rs.py list <file>` for upgrade.rs, post_upgrade.rs, update.rs, bin/launch.rs | 0 sites each |
| sockets | `find . -xdev -type s` | none |

Layer scenarios (17): first-run (record written, only ShellVersion read), unchanged (**0.01 s**, only ShellVersion),
unchanged + stamp (`loginctl disable-linger <uid>`, stamp gone), upgraded-active (one nl notification "Ubuntu is
bijgewerkt naar 26.10", no actions, record 26.10/51, compat/release/upgrade.json and the status file removed, snapshot
kept, linger off), out-of-date-dock (en "GNOME 51 turned Win11 Shell off", exact body and actions,
`EnableExtension(ubuntu-dock@ubuntu.com)` exactly once, logged after the fake's ActionInvoked), inactive + enabled +
dismissed (extOff, no EnableExtension), inactive not enabled (no notification), error + later, state missing, state a
string (both extError), extensions-off (extsOn → keyfile `disable-user-extensions=false`, no GetExtensionInfo, no
EnableExtension), missing-parts (body "Na de upgrade ontbreekt: GTK 3, libsoup."; apt-cache policy/showpkg calls in
order; **one transient unit `app-win11-post-upgrade-parts-*`, `Slice=app.slice`**, the exact argv ending `install
libgtk-3-0t65 libsoup-3.0-0`, LANGUAGE nl:en, WIN11_PRESS_ENTER, XDG_ACTIVATION_TOKEN/DESKTOP_STARTUP_ID = the fake's
token, PATH = stubs, no `WIN11_UPDATE_*`, cwd `$HOME`, nothing ran directly), shell-absent-dpkg (2.25 s of retries,
`dpkg-query -W -f=${Version} gnome-shell`, "GNOME 51 …"), upgrader-running (nothing on the bus, no tool, record and
stamp kept), seam-refused (`WIN11_UPDATE_TYPO`: nothing), no-answer (8.06 s, no EnableExtension, record written after
the wait), lib-start-app-slice (the lib's ignored test `layer_start_goes_through_the_user_manager` in the layer: **one
unit `app-win11-release-upgrade-*`, mode fail, `Slice=app.slice`**, argv `[…/xdg-terminal-exec, --title=Ubuntu-upgrade,
--, …/sh, -c, WRAPPER, win11-gnome-upgrade, …/home/.run/win11-gnome/upgrade-status.json, fail, …/pkexec,
…/do-release-upgrade]`, LANGUAGE nl:en, WIN11_PRESS_ENTER "Druk op Enter om dit venster te sluiten.", cwd the fixture
home, the plain-spawn stub never ran).

## 5. Contract with GW6 (the snapshot `release` object, design §4.3)

**GW7 reads** from `release::status(shared)` (GW5's object, GW6's verdict): `state` = the verdict (`none`,
`checking`, `ok`, `warn`, `hold`, `ubuntuHold`, `blocked`, `unknown`), `allowAnyway` (bool: `releases.json`
allowAnyway of the offered codename; GW6 please set it in this object), `offered`, `version`, `codename`, `reasons`.

**GW7 sets** (in `compose`, every host, after GW2's `running` line):

| Field | Value |
|---|---|
| `state` | the card: `running` (any upgrader, or our start before it is seen) · `waiting` · `startFailed` · `ended` · `none` (our upgrade waits for its reboot: card hidden, hero restartRequired) · `updatesFirst` · `rebootFirst` (both only while offered and the verdict is ok/warn) · else the verdict unchanged |
| `verdict` | GW6's state under the overlay (for a local "retry"/"close" back to the offer) |
| `startRc` | the wrapper's rc for `startFailed` (126/127/other; `null` for the no-window case) and `ended` (or `null`); else `null` |
| `reasons` | `startFailed`: `[{code:"start_failed", message, key, params:{}}]` with key `update.release.err.noPassword` (126), `.wrongPassword` (127), `.refused` (other rc) or `.noWindow`; otherwise GW6's reasons untouched |
| `verdictReasons` | GW6's reasons, only when `reasons` was replaced |
| `heldNames` | `updatesFirst` when only held packages block: their names (for `relUpdatesFirst`); else `[]` |
| `attachable` | `running`/`waiting` and Ubuntu's screen session runs: show "Venster weer openen" only then |

`update.changed` carries `release.state` (the overlaid one) and `release.version`, as before.

**The page calls**: `update.release.start {}` for ok/warn; `{anyway:true}` only after the "Toch upgraden…" warning (hold
+ allowAnyway); answers `{started:true}` or `{started:true, already:true}`; refusals per `api-docs-gw7.md`
(`updates_pending` carries `data {count, held}`). `update.release.attach {}`. **startFailed "retry"**: call
`update.check` (it clears the shown record and reruns the checks) or render `verdict` locally; **ended "close"**: hide
locally; the host drops the record at the next `update.check` once the card has shown it. A mirror's page sees the same
fields (files only); only the owner records outcomes.

## 6. Decisions for the coordinator

1. The guard (§2.5): keep §3.7's list (D-G40, current tests) or also refuse `update.release.ubuntuHold` while the
   upgrader runs (§6.4 step 7). The latter changes two GW2 tests (unit + bus test), which GW7 does not own.
2. Accept D1 (`anyway`), D3 (attach needs the screen session, `attachable`), D6 (record lifetime) and D7 (only ok/warn
   start) for the as-built section.
3. Merge: `api-docs-gw7.md` into `api-methods.json` (WU-FIX). GW6's card must read `state`, `startRc`, `reasons`,
   `heldNames`, `attachable` as above and call `update.check` for startFailed's retry.
4. Installer tests: accept the `Case.tree()` default that leaves the autostart entry out (D13), or fold the
   `Autostart` comparison into the existing byte-for-byte cases (GE1c's files).
5. For Robin (D-G1b class): nothing found in GW7's area.

## 7. What Robin's R5 VM run must check (in addition to §11 R5)

* xdg-terminal-exec on 26.04 accepts `--title=` and Ptyxis shows "Ubuntu-upgrade"; the window gets focus (token);
  with a Ptyxis already open, the new tab/window still gets the command's environment (`WIN11_PRESS_ENTER`, `LANGUAGE`).
* `systemctl --user status 'app-win11-release-upgrade-*'` and `systemd-cgls` while it runs: the terminal's unit and
  Ptyxis's `vte-spawn-*.scope` are in `app.slice`, never `app-win11shell.slice`; pkexec's root child is under them.
* `loginctl show-user $USER -p Linger`: `yes` during the upgrade (and `~/.config/win11-gnome/update-linger-by-us`
  exists), `no` after a dismissed pkexec dialog (card "De upgrade is niet gestart", rc 126 in
  `$XDG_RUNTIME_DIR/win11-gnome/upgrade-status.json`, the terminal held with "Druk op Enter …") and after the
  post-upgrade check at the first login on the new release.
* A wrong password (127 → "Het wachtwoord klopte niet"), and answering "no" at Ubuntu's first question: note the rc
  in the status file and whether the card returns to the offer (rc 0) or says "gestopt" (rc ≠ 0, seen).
* The `/proc` detection on the real cmdline (`python3 … /tmp/ubuntu-release-upgrader-*/<codename> --mode=server
  --frontend=DistUpgradeViewText` and `SCREEN … -S ubuntu-release-upgrade-screen-window`): the card says "Ubuntu wordt
  bijgewerkt", "Venster weer openen" appears, attach asks for a password and resumes the screen session; at a long
  prompt (conffile) "Ubuntu wacht misschien op een antwoord" after 10 minutes.
* After the reboot: the autostart entry runs once (`journalctl --user -b | grep -i post-upgrade`), the notification is
  GNOME's (also with the Win11 extension off), "Ubuntu Dock aanzetten" enables the dock only on the click,
  `GetExtensionInfo`'s state really is a double on GNOME 51, ShellVersion is readable at autostart time (no 30 s
  wait), and `~/.config/win11-gnome/last-os-release.json` names the new release.
* Optional: remove a library on the VM (e.g. rename `libsoup-3.0.so.0`) before a re-login after changing the record:
  "Onderdelen installeren…" opens the held terminal with `pkexec apt-get install <names>` and apt asks Y/n.
* The sign-out-mid-dpkg case of §11 R5 decides GW7b.
