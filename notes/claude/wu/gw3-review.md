# GW3 review: the Windows Update page and its mock (adversarial, 2026-11-05)

Reviewer: an independent agent. I did not write GW3. Worktree `.wt/gw3` (branch `wu/gw3`, HEAD d666df54, nothing
committed). I changed no product file. I wrote only this note and scratch files under `.rt/review-gw3/` (dumps,
notes, strace log, crops). Every heavy command went through `tools/heavy.sh` with a `gw3-rv-*` tag. I ran no win11-*
binary, gnome-shell, D-Bus tool, apt or PackageKit, and I did not re-run `shoot.sh`, because it would overwrite the
step's own screenshots.

**Verdict: fix-needed.** The product code is close to the design and every done-when check I re-ran passes. Two
findings still need action:

* **B1 (blocker, in the tooling):** the headless Chrome that the step's new `shoot.sh` starts (and that
  `tests/i18n/run.sh` starts too) connects to the **real system bus**. strace shows it. The step's product code has
  no such path.
* **M1 (major):** "Nu opnieuw opstarten" depends on `shell.command power`. In the real host its own-confirm fallback
  can never run, and the prewarmed standalone Start very likely handles the same command as well.

## 1. Checks I ran (from W's root)

| Command | Result |
|---|---|
| `git status --short`, `git diff --stat` | 15 modified + 8 untracked paths, all on the GW3 list (plus the coordinator's `notes/claude/plan.md`). Nothing under `host/`, `extension/`, `scripts/`, `tools/i18n/` (except `glossary.tsv`) or `tests/gnome/`. `tests/update-ui/{shoot.sh,shoot.html,serve.py,sheet.py}` are scripts in the owned new directory |
| `tools/heavy.sh --tag gw3-rv-dump-nl -- tests/i18n/run.sh` | `nl dump equals baseline-nl.txt (22917 lines)`, rc 0 |
| `tools/heavy.sh --tag gw3-rv-dump-en -- tests/i18n/run.sh --lang en` | `en dump equals baseline-en.txt (22914 lines)`, rc 0 |
| `… run.sh --out .rt/review-gw3/dump-{nl,en}.txt --notes …` (tags `gw3-rv-dump-{nl,en}-out`) | byte-equal to W's baselines (`cmp`). Notes: 194 surfaces, 0 frames without report, console messages only on `taskmgr-dialog.restart-shell` (not GW3) |
| `python3 -I tests/i18n/blocks.py <HEAD baseline> <my dump> ""` (nl and en) | in each language: 1 changed block (`settings.update`, not accepted), 35 new blocks, 0 removed. The `settings.update` hunk is exactly the §3.9 rows ("Er zijn 3 updates beschikbaar" → "Updates beschikbaar", "Controleren" + "Software-updater openen" → "Alles downloaden en installeren") plus new-key lines |
| `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check.py` | 0 errors, 52 warnings. The 5 update.js warnings are pre-existing reset/about texts (`Terugzetten`, `Voorbeeldvenster`) |
| `python3 -B tools/i18n/ratchet.py` | `ui: ok (89 allowed occurrences, 0 still to convert)` |
| `python3 -B tools/i18n/check_ui.py --dict-diff tests/i18n/dict-nl-3b898e6.tsv` | 2925 → 3140 rows; 9 changed, 20 removed, 235 added; 0 not in BASELINE-CHANGES.ui, 0 stale |
| `node --check --input-type=module < F` on the 10 changed or new modules | all ok. Plain `node --check F` also fails on unchanged files such as `about.js` (no `package.json` with `"type":"module"`), so the workaround is fine |
| `python3 -B tools/ui-manifest.py --check`; `python3 -B scripts/verify-ui.py` | up to date, 181 files; OK |
| `python3 -B -m unittest tests/i18n/test_update_mock_rows.py -v` | 5 run, 4 OK, 1 skipped (`update.tsv` has no rows yet) |
| `python3 -B -m unittest tests/i18n/test_ge0a.py` | 21 run, OK, 2 skipped |
| `git diff vendor/ui/css` | only `settings.css`, hunks only in the "Update hero" block and the new "Update rows" block (lines 758-816) |
| `tools/heavy.sh --tag gw3-rv-strace -- strace -f -qq -e trace=connect,bind -o .rt/review-gw3/strace.txt tests/i18n/run.sh --only settings-update.H10,settings-update.H20` | rc 0. Chrome connects to the real system bus and tries an IPv6 address on the internet (B1). The live session bus is never touched: the session bus is only `@w11-i18n-…` |
| Screenshots | counted 36 nl + 36 en + 12 en-narrow + 5 nl-dark = 89 PNGs and 18 sheets. All are newer than the last code edit (shots 18:57-18:59; update.js 18:39, settings.css 18:47). I looked at all 18 sheets and at several full shots (§4) |

## 2. Findings

### B1 (blocker, tooling): Chrome in `shoot.sh` (and `run.sh`) reaches the real system bus and is not network-isolated

* Evidence: `.rt/review-gw3/strace.txt:57`:
  `1084556 connect(27, {sa_family=AF_UNIX, sun_path="/run/dbus/system_bus_socket"}, 29) = 0`. This is Chrome's D-Bus
  thread. Next line: the same thread connects to the private `@w11-i18n-…` session bus. Line 79:
  `connect(26, {AF_INET6, port 443, "2001:4860:4860::8888"}) = -1 ENETUNREACH`. That is Chrome's IPv6 route probe.
  It failed here only because the host has no IPv6 route. On IPv4 nothing stops a connect.
* Cause: `tests/i18n/private.sh:44-52` uses `env -i` and starts a private session bus. It never sets
  `DBUS_SYSTEM_BUS_ADDRESS` and has no bwrap layer, so libdbus falls back to `/run/dbus/system_bus_socket`. The
  step's new `tests/update-ui/shoot.sh:65-70` starts Chrome the same way, with no bwrap, no private `/run/dbus`
  and no `--unshare-net`. The only network guard is `--host-resolver-rules`, which covers DNS names but not literal IPs.
* `strings` on `/opt/google/chrome/chrome` shows `org.freedesktop.login1.Manager`, `NetworkManager(.Device.Wireless)`,
  `UPower` and `systemd1.Manager`. I did not trace which calls Chrome made, to avoid contacting the bus again.
* The page and the mocks cannot drive PackageKit or a reboot through Chrome. Still, this breaks the hard rule ("never
  contact the real system bus … from any test or tool") and matches item 1 of the checklist (a child without
  `DBUS_SYSTEM_BUS_ADDRESS`, `/run/dbus` exposed).
* The gap already exists in GE0a's `private.sh`. GW3 copied it into a new tool.
* Fix:
  * **GW3 (owned):** in `shoot.sh`, run serve.py and Chrome inside one
    `bwrap --ro-bind / / --dev /dev --proc /proc --tmpfs /run/dbus --tmpfs /run/user --bind <work> <work> --bind tests/update-ui/shots … --unshare-net --die-with-parent`.
    Loopback works inside a new network namespace. At minimum, prefix Chrome with
    `DBUS_SYSTEM_BUS_ADDRESS=unix:path=/nonexistent`.
  * **Coordinator / GE0a:** apply the same fix to `tests/i18n/private.sh` or `run.sh`.
  * Verify with the strace command above: no `/run/dbus` connect, no AF_INET/AF_INET6 connect except to 127.0.0.1.

### M1 (major, plausible): "Nu opnieuw opstarten" broadcasts `shell.command power`; the fallback is dead and the prewarmed Start probably handles it too

* Evidence:
  * `update.js:456-467`: the page falls back to its own confirm and `system.power.action` only when
    `shell.info.panel === false` or `delivered === 0`.
  * `shell.info` has no `panel` field. `host/src/services/shell.rs:143-146` computes
    `delivered = self.ctx.subscriber_count()`, which counts the subscribers of the Settings process itself. The page
    subscribes to `update.changed`, so `delivered` is always at least 1.
  * So in the real host the fallback never runs. With no panel (for example while the extension's fallback bar
    shows), the button does nothing.
  * Separately, `vendor/ui/js/shell/shell.js:529` loads the `power` module in the standalone Start, and `:539`
    subscribes it to `shell.command`. `power.js:99-102` then confirms and runs `system.power.action`.
  * Every host binds an event socket (`files/mod.rs:147-151`), and `shell.` is on the fan-out allowlist
    (`events.rs:339`). So the hidden, prewarmed Start very likely also gets the command and opens its own power
    confirm, which stays pending in a hidden window. With `power.confirm=false` it sends a second reboot.
  * `update.js` is the first cross-process sender of `shell.command power` in G: a grep finds no other.
* Fix:
  * Until GX1 or GW2 define a panel-addressed command, `restartNow()` should use the page's own confirm (the copied
    `rebootQ`/`rebootBody`/`reboot` keys) and then call `system.power.action {action:'reboot'}`. That is
    power.js's final call, and the §4.9 guard applies to it.
  * Alternatively, GX1 makes the standalone Start ignore `shell.command power` and defines a "panel received it"
    signal.
  * Verify in the GNOME harness (GW8/GX1): exactly one `systemctl reboot` in the stub log after H10's click, and no
    power dialog in the Start process.

### m1 (minor): the page depends on error codes that the design never defines

* Evidence:
  * `update.js:485-486`: Retry chooses check over install on the codes `network`, `offline` and `check_failed`.
  * `update.js:333`: `BAR_SEVERITY = {cancelled:'info', test_seam:'error'}`.
  * The design defines only `network` (§4.3 example), `cannot_cancel` (§4.3) and `test_seam` (§4.11). For the others
    it defines keys (`update.err.offline`, `.checkFailed`, `.cancelled`).
  * The step's decision D1 for GW2 lists snapshot fields but not these codes. If GW2 names a code differently, Retry
    after a failed check starts an install (or the reverse), and a cancel shows as a warning.
* Fix: match on `error.key` (C2 allows it, and §3.7 fixes the names), or add the exact code list to D1.

### m2 (minor): several §3.2-§3.4 text variants are never rendered on any surface

* Evidence, from grepping `.rt/review-gw3/dump-nl.txt` (each is 0 hits in an update surface): `progressDlNoEta`,
  `waitLockOther`, `restartDesc` (restart without a list), `offlineDescNoWhen`, `andMore` ("en N andere"),
  `rowGtk`/`rowSoup`/`rowGlib`, `cGtk`/`cGlib`/`cPixbuf`/`cCairo`/`cArchive`/`cSelf`, `noDetails`, `relRunningNoVer`,
  and the page's own `rebootQ`/`rebootBody`/`reboot`.
* The checker sees them statically. Their English, though, is never reviewed in context or in a screenshot, and
  `andMore`'s joining (`update.js:317-320`) and the no-version title clean-up (`:295`) run in no test.
* Fix: add about six scenarios:
  * H05 without `remaining`
  * H07 with `lockHolder:null`
  * H10 with `rebootPkgs:[]`, and one with 5 packages
  * H20 with all `shellParts` plus `self`
  * R06 with gtk/soup/glib rows
  * R05 opened on a row without details

### m3 (minor): "Voorbereiden…" stays during a real download when `totals.downloadSize` is unknown

* Evidence: `update.js:349-354` replaces `preparing` only when `total > 0`. The size comes only from the simulate's
  `DownloadSizeRemaining` (§3.3), which the apt backend may not report.
* Then the hero says "Getting ready…" for the whole download, while the rows show "Downloaden - 45%".
* Fix: when `!j.simulating` and no total is known, show an empty caption (or a percentage key) instead of
  `preparing`. Tell GW2 to always fill `downloadSize` once the download starts.

### m4 (minor): the hero button can be clicked twice while a write is in flight

* Evidence: `write()` disables the button (`update.js:441`). A `paint(false)` from an `update.progress` event during
  the call runs `btn.setDisabled(!!model.disabled)` (`:513`) and re-enables it.
* A second "Annuleren" or "Alles downloaden en installeren" then reaches the host and comes back as a `busy` or
  `stale` InfoBar.
* Fix: keep a `pending` flag in `updateSection` and OR it into `setDisabled`.

### m5 (minor): the metered or removals confirm can open a second time after the user answered

* Evidence: `paintConfirm` sets `confirmOpen = null` as soon as the user answers (`update.js:651-655`), before
  `update.confirm` returns.
* A structural reload in between (an `update.changed` event) still sees `job.confirm` and opens a new dialog with the
  same key (`:642`).
* Fix: remember the answered key (for example `answered = key`) and skip it until the snapshot's `job.confirm`
  changes.

### m6 (minor): after one row is opened, every reload asks for the full package list

* Evidence: `wantPackages = true` is set for good (`update.js:433`), and `load()` then always sends
  `{packages:true}` (`:421`). §4.3 says the page asks once, "so the regular payload stays under 8 KiB".
* Fix: ask for packages only in `packages()` and when an expanded row's `rev` moved.

### m7 (minor): glossary row "Opnieuw proberen → Retry" (design §8) is left out; Retry and Try again are mixed in one app

* Evidence: `tools/i18n/glossary.tsv` has 12 of the 13 §8 rows. `update.js:198` says "Retry", while
  `settings/common.js:38`, `bluetooth.js:138` and `power.js:77` say "Try again" for the same "Opnieuw proberen".
* The deviation is documented (D5). Fix: the coordinator decides in round 3. Until then, list it in the round-3
  glossary decision.

### n1 (note): screenshots are not excluded from git

`tests/update-ui/shots/` holds 13 MB of PNGs, and `.git/info/exclude` has no rule for it. D-G22 keeps screenshots
out of git. Add `/tests/update-ui/shots/` (the step's own D6).

### n2 (note): the H02 shot does not show the checking ring

The frozen indeterminate `ProgressRing` renders as one dot (`.rt/review-gw3/h02-crop.png`). The checking state can't
be reviewed from it. Fix: in `shoot.html`, set the ring's animation `currentTime` to mid-cycle instead of leaving it
at 0.

### n3 (note): mock API errors lose key and params

`mock/index.js:105` rethrows only `code` and `message`. The `key`/`params` that `mock/update.js` `apiErr` attaches
never reach the page, even though the comment in `mock/update.js:28-29` says they do. The page matches only on code,
so nothing breaks.

### n4 (note): mock write methods crash in H28

`update.install`, `confirm`, `cancel`, `repair` and `details` read `u.snap.*` without a null check
(`mock/update.js:435`, `:446`, `:462`, `:473`, `:485`). In H28 they throw a TypeError, which reaches the page as
`internal`. This is reachable only from the console.

### n5 (note): a host without update.* loses today's behaviour

The fallback (H28) drops today's count and the Check button. The real host has no `update.*` until GW2 merges, so
between the merges the page says "Kan niet controleren op updates". Today it shows "Er zijn N updates beschikbaar".
This doesn't matter if GW2a and GW3 merge together (D-G29). Otherwise, fall back to `system.updates`.

### n6 (note): the history stub states something false

The `update-history` placeholder says "Er is nog geen updategeschiedenis." on a real host that has history. This is
harmless while round 5 (GW4) lands before any deploy (R3 comes after Phase A).

### n7 (note): deviations the step documented

The key names (`checking` for §3.2's `checkingFor`, the `rs*` row states), the full-width "Technische details"
placement, the 5 extra keys and the four extra snapshot fields (D1) are all documented in `gw3.md` §6/§8.

## 3. Conformance (§10 GW3 row, §3.1-§3.6, §3.8, §3.9)

| Requirement | Status |
|---|---|
| Every §3.2 hero phase in mock nl and en, incl. releaseRunning (H25), foreignOwn (H26), keptBack (H16) | met; caption variants partly missing (m2) |
| §3.3 rows: order, titles, caption, securityTag, 7 row states, 160 px bar only while moving, error bar, per-row Retry, details, `offeredLater`/`keptBackRow`, "Wat is er nieuw" via `update.details`, https-only links via `apps.openUri` | met (gtk/soup/glib not rendered, m2) |
| §3.4 hero restart and the Win11 InfoBar, hidden while a reboot is pending except for webkit/jsc (H27) | met |
| §3.2 "Nu opnieuw opstarten" with its own confirm when there is no panel | **not met in the real host** (M1) |
| §3.5 / §3.6 stubs with final exports; Meer opties list; the updater stays reachable in Geavanceerde opties | met |
| §3.8 `system.updates.rebootRequired` in the mock | met |
| §3.9 Dutch changes only for the listed keys; BASELINE-CHANGES.ui lists them | met: dict-diff 0 unlisted, block diff = §3.9 rows plus new keys |
| BASELINE-ADDITIONS.ui = the new keys (112 rows) | met (the test checks it) |
| C2: matching on code or key, never on message | met. The codes themselves are not defined in the design (m1) |
| en-GB, Windows 11 24H2 wording, ’, sentence case | met. Every new en text equals the design's tables. I found no straight apostrophe in user-facing en text and no US spelling |
| CSS only in the update blocks | met |
| Glossary §8 | 12 of 13 (m7) |
| Screenshots reviewed against 24H2 | done from memory, headless Chrome only. No WebKitGTK harness shots (the step says so) |

## 4. Screenshot review (mine, from memory of Windows 11 24H2 Settings › Windows Update)

* **Matches 24H2:**
  * One accent action per state, right-aligned.
  * Hero title in subtitle size with a secondary caption.
  * Rows as stacked 66 px cards: title and caption on the left, status on the right, "Downloaden - 45%" over a 160 px
    bar (H05, R01).
  * "Meer opties" holds Updategeschiedenis (history glyph) and Geavanceerde opties (gear, description).
  * Breadcrumb "Update & recovery › Advanced options". Dark mode is correct (H10, H12).
* **Design-sanctioned differences:** the 64 px disc behind the glyph; rows are expanders with a chevron; the
  "N updates worden later aangeboden" line; Cancel during the download.
* **At 800 px in English:** no clipping. The hero button wraps under the glyph at the left edge (H09, H27), which is
  the existing `.st-upd` flex-wrap behaviour, not new. Row statuses wrap under the title as intended.
* Nothing in the look needs a fix before GW8. n2 limits how well the checking state can be reviewed.

## 5. Safety summary (checklist item 1)

* **GW3 product code:** on load the page calls only `update.status`, which is read-only. Every write runs on a click.
  Dialogs answer `update.confirm` only when the user answers. No auto-check on page open.
* **Mocks:** they run only with `?mock=1` or without a host token.
* **Hard rules:** no Dutch literal outside dictionaries (the mock's "Onbekende functie" copies `mock/index.js`). No
  frozen file was edited.
* **Tooling:** only issue is B1.
