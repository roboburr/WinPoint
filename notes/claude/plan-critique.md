# Plan critique (GE0e): adversarial review of `notes/claude/plan.md`

2026-11-05, read-only. Inputs: `notes/claude/plan.md` (93 lines), `notes/claude/gnome-map.md`, the eight reports in
`notes/claude/map/`, spot-checks in G's source, H's `notes/english-option-design.md` and `tests/i18n/`, and the
machine's earlyoom, memory and disk state. Nothing was built, launched or installed. No file in G was changed apart
from this one. G = `/home/robin/Documents/Codex/2026-11-01/chat/outputs/win11-gnome`; H = the frozen Hyprland tree.
Paths without a prefix are relative to G; line numbers are at `3b898e6`.

Severity: **Critical** = would break Robin's session, lose Dutch fidelity unnoticed, or stall a whole round.
**High** = likely rework or a silent gap. **Medium** = a real gap with a workaround. **Low** = hygiene.

## Summary: the eight changes that matter most

1. **The plan runs parallel agents in one working tree, and its own git-status rule makes each of them stop** (F1). In round 2, each agent's `cargo check` also compiles the others' half-finished edits (F2). Give every step a path allowlist, and give each round-2 agent a git worktree.
2. **Three round-1 items have no single owner:** the desktop-entry rewrite (F3), the extension error mapping (F4) and `scripts/verify-ui.py` (F5). Three couplings need a written contract before round 1: `setTitle`, `appName ""` and the `ApiError` shape (F6).
3. **D-G1, D-G2 and D-G5 decide what GE1a puts in Dutch and English.** `proposed/` already contains the endonym and the count-of-1 changes. Ask Robin now in one question, or GE1a holds those hunks back (F30).
4. **Round 2 needs a gate that does not exist yet** (F8): a frozen i18n API, every stem file pre-created, a converter tool, Rust checker rules and one reviewed pilot conversion. Without them, about 463 literals get retyped by four agents.
5. **The Rust Dutch guard is under-specified** (F12, F13). A frozen file keyed by file:line breaks on the first edit, and a Dutch-word heuristic misses unmarked Dutch. Freeze every non-test literal as decoded bytes, and make the three Rust oracles part of every round-2 agent's done-when.
6. **The headless harness has three blockers and two blind spots** (F23-F27):
   * Unix socket paths inside G go over the 108-byte limit.
   * The guard list is missing variables.
   * Mocks would edit `vendor/ui`.
   * Rewriting `settings.json` skips the real switch path.
   * "Dutch pixel-identical" cannot be tested against live data.
7. **earlyoom here never kills WebKit, `win11-app` or `win11-panel` processes, and it avoids `gnome-shell`** (F32). If a test harness runs away, Robin's own largest app is what gets killed. The memory plan needs a self-watchdog, a check made inside the lock, and an offline flag that `build.sh` does not pass (F33, F34).
8. **There are packaging and machine-history gaps:**
   * D-G8's `i18n/host/` sits outside the staged tarball (F17).
   * `install.py` hard-codes the extension's file list (F18).
   * A reinstall resets the GNOME names to Dutch (F19).
   * The extension reads H's old `~/.config/win11shell/settings.json` on machines that still carry H, the NucBox and very likely this PC (F20).

## A. Order and dependencies

**F1 (Critical): the "Codex check" stops every parallel agent.**
* **Evidence.** The rule is `plan.md` ground rules, "a change you did not make means someone else works here: stop and report it". Round 1 runs GE1a, GE1b and GE1c in one tree, and round 2 runs four agents in one tree.
* **Already happening.** During this review, `git status` changed from clean to `?? tests/` and `?? tools/`, written by GE0a and GE0b. A strict agent would have stopped.
* **Change.** Give each step an explicit path allowlist (the tables in section H). Rule: "Stop only when a path **outside** the union of the current round's allowlists changed, or a tracked file outside your own list changed." Also:
  * gitignore the sandbox state. `tests/rust-sandbox/{home,run,cache,config,data,logs}` exists now and would be committed by `git add tests/`.

**F2 (High): round 2 in one working tree means shared compile breakage.**
* **Evidence.** `cargo check` builds the whole `win11-host` crate. Agent A's half-done edit breaks agent B's check, and B cannot tell the difference. The heavy lock serialises cargo runs, but it does not serialise edits.
* **Change.** The coordinator creates one git worktree per round-2 agent, under `G/.wt/<agent>` (gitignored, inside G), on its own branch. The agents share one `CARGO_TARGET_DIR` (deps are reused; the crate itself rebuilds per path). The coordinator merges. Disk has room for 4 checkouts (F35).
* **Cheaper fallback.** Two agents at a time, each running `cargo check` only after its own edits are complete.

**F3 (High): the desktop-entry rewrite (D-G4) is owned by nobody and races.**
* **Evidence:**
  * D-G4 says "the host rewrites the user-local entries on a language switch", but the plan lists it under GE1c (extension and installer). The host is GE1b's domain, and GE1b is also the only step that edits `lib.rs`.
  * Every Win11 process sees the switch: `apps/mod.rs:221-244` polls the language every 150 ms in each process that runs the apps service. A naive hook gives N concurrent writers.
  * A rewrite also fires `gio::AppInfoMonitor` (`apps/mod.rs:214-215`) in every process, so `apps.changed` is sent twice. This is harmless but worth knowing.
  * `install.py:110-131` generates the entries with Exec paths, StartupWMClass and MimeType, and without a marker. A second generator in Rust would drift from it.
* **Change:**
  * **Single writer:** the `win11-panel` process, which is unique. Debounce 1 s. Run it on panel start, to repair a switch made while the panel was down, and on a language change.
  * **Rewrite only the `Name=` and `Comment=` lines** of the 10 files `io.github.win11shell.{…}.desktop`. Leave every other line byte-identical. Then run `update-desktop-database` only if it exists.
  * **One shared table,** `host/src/desktop-entries.json` (id → nl/en name and comment). Rust reads it with `include_str!`; `install.py` reads it from the tree.
  * **Owners:** GE1b owns the Rust side, GE1c owns `install.py`.
  * Note that the Start menu does not depend on this, because own-app names come from `apps/catalog-data.json` `ownApps` (`catalog.rs:198`). Only GNOME's own surfaces do.

**F4 (Medium): the placement of "error mapping by code" (GE1c) is ambiguous and causes overlap.**
* **Evidence.**
  * `panelBridge.js:39` `failure(code, message)`. The codes are categories: `session_unavailable` has at least 3 different messages (`:296,:382,:516`) and `not_found` at least 2 (`:319,:402`).
  * `gnome.rs:17-27` passes the message on.
  * `shell.js:459-461` shows `error.message`.
  * Mapping by code in the UI would lose detail, and it would put GE1c into GE1a's `vendor/ui`.
* **Change.** The extension renders its own failure texts from its inline `{nl,en}` table, using the same `lang()` it gains for its labels. Then no UI or host file changes. The host fallback `"De GNOME-actie is mislukt"` (`gnome.rs:25`) goes to round-2 agent R2-D.

**F5 (Medium): `scripts/verify-ui.py` has two editors.** GE1a needs a manifest generator, and GE1c translates the
messages at `verify-ui.py:19,25`.
* **Change.** GE1a writes a **new** `tools/ui-manifest.py`, which regenerates `vendor/ui-sha256.json` sorted and deterministic, and does not touch `verify-ui.py`. GE1c alone owns `scripts/**`.

**F6 (High): cross-layer couplings have no contract.**
* **Window titles.** `appframe.js:252-256` sends `setTitle` only when `window.parent !== window && !nativeHandler()`, so never in a native window. `window.rs:463-466` already handles a `setTitle` message.
  * Using wry's title-changed handler instead (rust-apps item 4) would copy `document.title`. That carries per-page titles, for example `settings/main.js:221` "`${pageTitle} – ${t('title')}`" and `taskmgr/main.js:191`, into the GTK title, the taskbar previews and Alt+Tab. Windows 11 shows "Settings" and "Task Manager" there.
  * Taskmgr, Alles, Disks and Explorer never call `frame.setTitle` on a language switch; only Settings, Default apps and the picker do (`settings/main.js:292`, `defaultapps/main.js:66`, `picker.js:88`).
* **`appName ""`.** proposed `toasts.js`/`notifcenter.js` and `notifications.rs:361`. Items already stored with "Melding" need mapping when they are loaded.
* **The `ApiError` JSON shape**, and how `key`/`params` reach `makeError` in `bridge.js` and `hostcall.js`.
* **Change.** Before round 1 starts, the coordinator writes `notes/claude/contracts.md` covering:
  * titles: `frame.setTitle` → IPC `setTitle` with the app's own title (not `document.title`), and every app calls it on `onLangChange`. Rust sets the initial nl/en title from a table at window creation;
  * `appName`: `""` plus the load-time mapping;
  * the envelope: `{code, message, key?, params?}`;
  * `X-Shell-Lang`: sent by the UI and ignored by the host in v1 (F9).

**F7 (High): GE0c is not independent of GE0b, and GE1c must not build mid-round.**
* **Evidence.** `install.py:42-50` needs all 12 binaries. GE0c "installs a build into a private prefix", and the only build is GE0b's. In round 1, GE1c's installer tests would otherwise rebuild a tree that holds GE1b's half-done Rust.
* **Change.**
  * GE0c starts after GE0b's build exists.
  * GE0b copies the 12 baseline binaries to `tests/build/3b898e6-debug/` (about 340 MB). They are the "before" side for GE0c, GE1c and round 3.
  * Always pass `WIN11_BUILD_DIR` explicitly, because the stale `G/bin` wins otherwise (`install.py:42`).

**F8 (High): there is no gate between round 1 and round 2.** "Round 2: Rust text conversion (≈463 sites, four agents)" has no precondition.
* **Change.** Round 2 starts only when GE1b has delivered these, and the coordinator has reviewed them:
  1. a frozen `i18n.rs` API (`Lang`, `tr`/`trf`/`trn`, `ApiError::tr`);
  2. **every stem file created** (empty) **plus the complete `include_str!` list**, so no round-2 agent edits `i18n.rs` or `lib.rs`;
  3. the converter `tools/i18n/extract_rs.py` (F13);
  4. the Rust checker rules (key exists, nl/en placeholder parity, plural completeness, stem = file);
  5. one pilot area converted end to end: the `core` stem (`runtime.rs` envelopes and `unknown()` `:517-520`, `services/mod.rs:87,116`), with oracles green.

**F9 (Medium): GE1b is overloaded, and part of it buys nothing in G.**
* **Evidence.** GE1b lists the core, the envelope, `X-Shell-Lang`, the reader, the 4 `_STR` tables, the caches, "Melding" and the titles: 15+ files.
  * G has no D15 race (gnome-map 7.1 check 1).
  * GE0c switches language through `settings.json`, not `?lang=`, and the UI dump runs in mock mode with no Rust host. So the host side of `X-Shell-Lang` serves no current test.
  * It adds a thread-local that can leak into `LazyService` initialisation and shared caches on request threads (rust-services, Risks).
* **Change:**
  * **D-G7: the host ignores `X-Shell-Lang` in v1.** The UI keeps sending it, as `proposed/` does, and that is harmless.
  * **Move the 4 `_STR` JSON tables and the language-keyed caches to the round-2 owners of those files,** or keep the JSON tables and let the checker read them (my default). They are already bilingual, and their nl was verified equal to H.
  * Converting them changes Dutch where H made `.one`/`.other` rows (F14).

**F10 (Medium): English layout widths come too late (round 3).**
* **Evidence.** The en UI is complete at `3b898e6` (2,638 keys at parity). `settings.css` alone has 14 `nowrap`/ellipsis rules (ui-settings). Codex's real taskbar search input. No width was measured anywhere.
  * H's dump runs in headless Chrome (`H/tests/i18n/run.sh:2-4`), whose font metrics are not WebKitGTK's.
* **Change.**
  * GE0c adds an overflow audit in WebKitGTK through `WIN11_DEV_SCRIPT`: every element with `text-overflow: ellipsis` or `white-space: nowrap` where `scrollWidth > clientWidth`, reported per surface for nl and en at `3b898e6`.
  * GE1a (or a small GE1d) fixes only the en overflows, under the pixel rule of F26.

**F11 (Low): D-G14 can move.** Window-memory hardening (`lib.rs`, `window.rs`, `window_state.rs`) does not overlap with
the update rounds 4-6 (`services/update*`, `events.rs`, `api-methods.json`, `update.js`). It can run parallel to round 4
instead of in round 7. It must not run in round 1, because GE1b owns `lib.rs` and `window.rs` there.

## B. Dutch drift

**F12 (Critical): the frozen Rust file (GE0b) needs a stable identity and full coverage.**
* **Evidence.** The plan says "user-visible string literals with file:line, classified". Line numbers change with the first edit.
  * The ≈463 count comes from a Dutch-word heuristic (rust-services ±5 %; rust-apps: "Dutch text with no marker words may be missed").
  * Unmarked Dutch ("gebruiker", "{}-bestand", "kopie") was found only by hand.
* **Change.** Freeze **every** non-test string literal in `host/src/**/*.rs` plus the nl values of the JSON tables (`types.json`, `launcher-names.json`, `catalog-data.json`, the 4 `_STR` files). Store the decoded bytes, keyed by (path, ordinal within file, sha256), with a classification column.
* **The drift check, run by each round-2 agent and in round 3.** Each frozen literal must meet one of three conditions:
  1. it is still present byte-identical in the same file; or
  2. it equals the nl value of a TSV row that the converter's mapping file assigns to that site (after the recorded `{}` → `{name}` placeholder map); or
  3. it is listed in `tests/i18n/BASELINE-CHANGES.rust` with a reason.

  Also add generated nl render tests: for each converted site, `trf(Nl, key, sample)` must equal the frozen bytes with the same sample values.

**F13 (High): there is no converter step, so the literals get retyped.**
* **Evidence.** The oracles hold curly quotes byte for byte. `disks/reference-tests.json` expects "‘Fixture’ is nog in gebruik". The conversion also has to handle `format!` positional arguments, `{:?}` quoting, trailing spaces and sentences built from parts (`files/properties.rs:158` "Alle van het type ").
* **Change.** GE1b delivers `tools/i18n/extract_rs.py`:
  * it decodes Rust escapes;
  * it turns `format!` arguments into named placeholders and records the mapping;
  * it writes the TSV rows, taking nl from G and en from H only on an exact nl match;
  * it emits the call-site patch;
  * it refuses `{:?}` and width/precision specifiers for manual handling.

  Round-2 agents run it per file and review the output; they do not type nl.

**F14 (High): H rows that change Dutch.**
* **Evidence:**
  * 36 of H's 528 reusable rows do not match G verbatim (port-path W10).
  * H made 4 sysmon keys `.one`/`.other` plurals. The `_pl` ad-hoc plural is at `sysmon/health.rs:303-307`. Adopting them changes nl for n == 1.
  * About 94 H rows come from the unreviewed round 2.
* **Change.** Never copy an H TSV file. The converter is the only way a row enters G. Every plural adoption that changes nl is a `BASELINE-CHANGES.rust` row and falls under D-G1.

**F15 (High): there is no never-translate list for Rust.**
* **Evidence.** Several kinds of literal must stay as they are:
  * `settings_effects.rs:14-19`: `CAP_LINE`/`THEME_LINE` are Dutch comment lines written into the user's GTK CSS between machine markers. Today's text even names H's `install.sh; ./uninstall.sh`. H D11 says machine markers stay byte-identical.
  * `index/query.rs:273-847`: input words.
  * Daemon-English matchers (`disks.rs:566-592`, `bluetooth.rs:1213-1224`, `tray/helpers.rs:484-497`).
  * Dutch logs (H D10).
  * `api-methods.json` docs.
* **Change.** GE0b's freeze marks these `keep` with a reason in `tests/i18n/allow-host.txt` before round 2. The converter refuses them.

**F16 (Medium): Rust messages quote UI labels.**
* **Evidence.** `files/actions.rs:348` names the UI button "Starten toestaan". `settings_effects.rs:16` names the path "Instellingen > Persoonlijke instellingen > Kleuren".
* **Change.** Add a checker rule: a quoted label in a Rust en row must exist as an en value in the UI dictionaries, or be allowlisted.

## C. Packaging and installation

**F17 (High): D-G8's location breaks the source tarball.**
* **Evidence.** `package.py:30-31` stages only `scripts extension host vendor` plus root files. A catalogue under `G/i18n/host/` embedded with `include_str!("../../i18n/…")` is missing from the archive, so building from it fails.
* **Change.** Use `host/i18n/<stem>.tsv` (inside the crate) and point the ported `check.py` at it. Or, second best, add `i18n` to `package.py`. Decide this in round 1, not round 7.

**F18 (Critical once deployed): `install.py` hard-codes the extension's files.**
* **Evidence.** `install.py:104` copies exactly 9 named files. A new module, for example `extension/i18n.js` imported by `extension.js`, would not be installed. The extension would then fail to import after the re-login: no Win11 taskbar, and ubuntu-dock is disabled.
* **Change.** GE1c keeps new code inside the existing files, or updates the list and adds a test: installed set = `git ls-files extension` minus the schemas. GE0c's install path must catch a mismatch.

**F19 (High): a reinstall resets the GNOME names to Dutch.**
* **Evidence.** `install.py:110-131` always writes Dutch `Name=` and a Dutch developer `Comment=` (`:126`). After an update, an English user sees Dutch names in GNOME until the next switch (or until the panel's F3 repair).
* **Change.** At install time, `install.py` writes `Name`/`Comment` in the language from the existing `settings.json` (missing → `--lang` → nl), using F3's shared table. Test: install, switch to en, reinstall, and the names stay English.

**F20 (High): H's old install on the NucBox, and very likely on this PC.**
* **Evidence:**
  * The extension reads `~/.config/win11shell/settings.json` when `~/.config/win11-gnome/settings.json` is missing (`extension.js:20-31`).
  * The Rust host has no such fallback (only the Python reference does, `backend.rs:188`). It creates `settings.json` only on a real change: `settings.rs:927-941`, where setting the default does not create the file.
  * So on a fresh G install over H, the extension takes its language and Start alignment from H's file while the host uses nl.
  * The new directory monitor also has nothing to watch until `~/.config/win11-gnome/` exists.
  * H's installer also left these behind:
    * 5 `win11shell-*.desktop` entries (`H/config/applications/`): duplicate names in GNOME's grid;
    * a `mimeapps.list` default for folders to H's Explorer (`H/install.sh:1096-1134`);
    * a GDM session entry and systemd drop-ins.
  * This PC has H's interim UI in `~/.local/share/win11shell/ui` (port-path F1), so the same probably applies here (not checked; those paths are off limits).
* **Change:**
  1. The extension takes its **language** only from the win11-gnome file (missing → nl). It monitors `~/.config`, or creates the directory.
  2. On a first install, if no win11-gnome `settings.json` exists but H's does, the installer copies H's `language` into a new file (H D13 spirit).
  3. Add a read-only inventory script, `tools/h-remnants.sh`, that Robin runs on each machine. It lists H's desktop entries, mime defaults, session entry, drop-ins and payload. Plus a Robin decision on the clean-up (gnome-map §6.10).

**F21 (Medium): packaging items the plan does not list.**
* `package.py:36` names a missing `VALIDATION_PROCESS_MEMORY.md` and leaves out `VALIDATION_RESOURCES.md`.
* The version lives in 3 places: `host/Cargo.toml`, `metadata.json` `version`/`version-name`, and the README.
* The uninstaller is not installed.
* There is no runtime library preflight; WebKitGTK 4.1 on a stock 26.04 is unverified.
* The archive grows by the 81 MB icon tree.
* `vendor/host` is still required by `install.py:51-53`.
* The Windows Update root helpers (D-G12 D4) and the post-upgrade XDG autostart entry (G-W7) are new installed files.
* **Change.** List all of these under round 7, and decide the root-step mechanism in round 4, not at packaging time.

**F22 (Medium): re-login and the live deploy are not plan items.**
* **Evidence:**
  * Every change to `extension/*`, `metadata.json` or the schema needs Robin's re-login (gnome-map §5).
  * The live `~/.local/bin/win11-panel` (sha `17437f50…`) matches no record.
  * `uninstall.py:21-31` writes GSettings even with `--files-only`.
* **Change.** Round 7 gets an explicit Robin-only step, prepared by an agent and approved by Robin:
  * back up `~/.local/bin/win11-*` and `~/.local/share/win11-gnome`;
  * install with `WIN11_BUILD_DIR=target/release`;
  * one re-login that covers all extension changes;
  * a rollback recipe.

  No agent ever runs `install.sh` or `uninstall.sh` outside the harness.

## D. Harness and test gaps

**F23 (Critical for GE0c): Unix socket paths inside G go over the limit.**
* **Evidence.** `sun_path` holds 108 bytes. G's root is 64 characters. With the plan's `tests/gnome/run/<stamp>/runtime`:
  * `…/runtime/wayland-win11-test` is 120 bytes;
  * `.lock` makes it 125;
  * `…/runtime/bus` is 105 bytes.
* **Already happening.** GE0b already put its bus socket in `/tmp` (`tests/rust-sandbox/session-bus.conf`: `unix:dir=/tmp/w11-ge0b-…`), against the letter of the no-/tmp rule.
* **Change.** Either use a short runtime root `G/.rt/<n>` (≈ 70 bytes before the socket name), gitignored, mode 0700, with an assert that every socket path is under 100 bytes, or allow `/tmp` explicitly for socket directories only. Sockets are ephemeral, so the reboot argument does not apply.

**F24 (High): the GE0c guard list is incomplete.**
* **Evidence.** `plan.md` GE0c checks only these: `/run/user/1000/bus`, `XDG_RUNTIME_DIR`, a live `WAYLAND_DISPLAY`/`DISPLAY`, and a non-keyfile backend. Codex's first harness pointed `WIN11_REFERENCE_SETTINGS`/`WIN11_REFERENCE_INSTANCE` at live files and kept the real HOME and `~/.local/share` in `XDG_DATA_DIRS` (extension-packaging (d)).
* **Change.** Also refuse when:
  * `NO_AT_BRIDGE` is not 1 or `GTK_A11Y` is not `none`;
  * `HOME` is the real home;
  * `XDG_DATA_DIRS` contains `~/.local/share`;
  * any of `WIN11_REFERENCE_SETTINGS`, `WIN11_REFERENCE_INSTANCE`, `WIN11_GNOME_CONFIG_DIR`, `AT_SPI_BUS_ADDRESS` or `XDG_SESSION_ID` is set;
  * the patched `unsafe_mode` extension copy is outside the private `XDG_DATA_HOME`.

  Also:
  * write the keyfile `$XDG_CONFIG_HOME/glib-2.0/settings/keyfile` directly instead of calling `gsettings set`, so no agent has to judge whether a `gsettings set` is allowed;
  * never `--include-ignored`. `process_scope.rs:514` is an ignored test that creates units "on the actual user manager".

**F25 (High): the UI dump harness port touches `vendor/ui` and `/tmp`, and misses E1-only dialogs.**
* **Evidence:**
  * The mocks live in `vendor/ui/js/mock/*` (product code, hashed). "Add mocks for the 18 G-only methods" would therefore change `vendor/ui` in round 0, and the baseline would then not be `3b898e6`.
  * H's `run.sh` makes its work directory with `mktemp -d ${TMPDIR:-/tmp}` and runs headless Chrome outside any private bus or HOME.
  * Dialogs reachable only through E1's `?open=` hooks (Explorer context menu and conflict dialog, Taakbeheer dialogs, the launcher-trust dialog) cannot be in a `3b898e6` dump. After GE1a they appear as "new blocks" with no Dutch reference.
  * **This already happened during this review.** At 13:19, a round-0 agent (probably GE0a; the new comment refers to `tests/i18n/run.sh --notes`) edited `vendor/ui/js/mock/index.js` (+5/−1: a `console.warn` before the `unknown_method` throw). `python3 -B scripts/verify-ui.py` now exits 1 with `"changed": ["js/mock/index.js"]`. Dutch output is unchanged, but product code changed in round 0, and `package.py:17` would refuse to package.
* **Change:**
  * Serve an **overlay** (`tests/i18n/overlay/js/mock/*`) from `serve.py`; `vendor/ui` stays untouched.
  * Wrap Chrome in the same sandbox (private HOME, `dbus-run-session`, `NO_AT_BRIDGE=1`), and point `TMPDIR` inside G.
  * Record the baseline from `git archive 3b898e6` into `tests/i18n/base-3b898e6/`, so it can be reproduced later. Record the Chrome version (Chrome updates through its apt source).
  * Add a **dictionary-level nl snapshot** (`jsdicts.mjs` → `tests/i18n/dict-nl-3b898e6.tsv`: key → nl bytes, every dictionary). After GE1a, every changed nl value must be in `BASELINE-CHANGES`, reachable or not.

**F26 (High): "Dutch pixel-identical" has no feasible test.**
* **Evidence.** Real-host GE0c screenshots carry live data: the clock, CPU graphs, Wi-Fi names, notifications and the apps list. Pixel equality across runs cannot hold.
* **Change.** Run pixel comparisons on the **mock UI inside WebKitGTK** (Codex's `W/start-corners-test/webkit.py` pattern), with `freeze.js` (fixed clock, seeded random), at a fixed size and scale. Use real-host screenshots only for review by eye and for overflow. Provide a pixel-diff tool in `tests/gnome/` with a mask list.

**F27 (Medium): the switch path in GE0c is not the user's.**
* **Evidence.** "Switches language by rewriting the private `settings.json`" skips `settings.set`, which sends the invalidation datagram (`events.rs:160-169`). The peers then fall back to the 150 ms poll. The store writes by rename (`settings.rs:351,375`), and the extension monitor needs `WATCH_MOVES`. An in-place rewrite tests a different event.
* **Change.** Test both paths:
  * the Settings dropdown driven through `WIN11_DEV_COMMAND`;
  * an atomic rename of the file.

  Assert the time until each surface and the extension shows the new language.

**F28 (Medium): scenarios no step owns.**
* **Hidden preloaded Start.** It is `--preload` and realized but unmapped (rust-apps (b)). Its `onLangChange` is synchronous (`startmenu.js:709-713`), but the app list refetch is asynchronous. Scenario: switch while Start is hidden, then open it and capture the first frames.
* **Explorer tabs, tab transfer and an open picker during a switch;** Alles while it is indexing; Taakbeheer and Schijven with an in-flight call. These are the ones `proposed/` changes semantically (port-path F4).
* **Window titles.** Read them through Eval: `global.get_window_actors().map(a => a.meta_window.get_title())`.
* **GNOME names after the desktop rewrite:** `Shell.AppSystem.get_default().lookup_app(id).get_name()`.
* **The extension:** labels, accessible names, the Ctrl+Alt+Tab group, the clock, notifications and the fallback bar after killing the private panel.
* **The installer:** a private-prefix install proof (path with spaces, `%` and `$`), `--lang`, a reinstall that keeps the language, and F18's file-set check.
* **Rust en:** key exists, placeholder parity, plural completeness, and one en render test per stem.
* **Performance.** Every `ctx.language()` call is a full settings read under the lock (`services/mod.rs:62-69`). No `tr()` with `ctx.language()` inside the process-list loop. Measure the `sysmon` process list before and after.
* **Change.** Name an owner and a done-when for each. Most belong to GE0c (the capability) and to the step that changes the behaviour (the assertion).

**F29 (Low): the Rust oracles are mentioned but not wired in.**
* **Evidence.** `services/system/reference-tests.json` (475 cases, all nl), `shell/…` and `disks/…` (8 + 8 tests) are active, not ignored. They are the strongest automatic Dutch guard.
* **Change.** Every round-2 done-when includes them: `cargo test --offline --locked -j2 reference oracle` in the sandbox.

## E. Decisions: which are Robin's

**F30 (High): D-G1 is presented as settled, but it is Robin's.**
* **Evidence:**
  * H's decisions are "Defaults in bold. All of them can be revisited" (`H/notes/english-option-design.md:162`). I found no record in H's notes that Robin accepted D6 (the endonym) or the count-of-1 changes.
  * gnome-map §6.1 suggests the opposite default for the endonym: keep "Nederlands".
  * The ground rule says "Dutch stays default and byte-identical".
* **Change.** Split D-G1 into:
  * **(a) bug fixes:** "1 proces", "1 kern", "1.000 items", "1 item kon niet …". Default: accept, and tell Robin.
  * **(b) cosmetic:** "Nederlands (Nederland)", "Afbeelding", the Alles `hintHidden`/`trashFailedBody` wording. Default: hold back.

  Ask Robin once, together with D-G2 and D-G5, before GE1a starts, because these hunks are inside `proposed/` (`time.js`, `alles/i18n.js`, the plural objects).

**F31 (Medium): the classification of D-G1..D-G14.**

| # | Whose | Why / change |
|---|---|---|
| D-G1 | **Robin** | Changes Dutch text (F30) |
| D-G2 | **Robin** for the Dutch side | Bug-like, but it rewrites Dutch. Ask together with whether to hide the Clipboard page instead (`clipboard.list` is always unsupported, `clipboard.rs:30`). The Rust half (`power.rs:154`, `display.rs:86`) is assigned to no step: give it to R2-A |
| D-G3 | Coordinator | Add the exception: the installer's first-install guess (H D4/D13) reads the locale once, then stores the choice |
| D-G4 | Coordinator (mechanism), Robin informed | Sound for both machines (no `[xx]` keys, so this PC's en_US and the NucBox's nl both show the plain name), but see F3 and F19 for the owner and the reinstall |
| D-G5 | **Robin** | Product naming (map §6.4); it affects `panelBridge.js:387` `'Everything'`, `window.rs:193`, `alles/main.js:421-423` |
| D-G6 | Coordinator | Specify the source: `frame.setTitle`, not `document.title` (F6) |
| D-G7 | Coordinator | Change: the host ignores the header in v1 (F9) |
| D-G8 | Coordinator | Change the location to `host/i18n/` (F17); keep the `_STR` JSON in v1 (F9) |
| D-G9 | Coordinator | English only, no Dutch effect |
| D-G10, D-G11, D-G13 | Coordinator | Fine |
| D-G12 | **Robin** (D1-D6) | It changes another app's settings (D2: `com.ubuntu.update-notifier`), installs root files (D4) and picks the upgrade path (D1). The plan's "blocks with 'Wacht op een compatibele Win11-versie'" does not match the updates.md D3 default, "Ubuntu 26.10 is onderweg (hold) with 'Toch upgraden…' behind a warning". Pick one wording |
| D-G14 | Coordinator (own apps); **Robin** (all apps) | As written |

**Missing decisions, to add:**
* en-GB (H D1).
* `settings.language` takes only nl/en, with an unknown value or the object form (`settings.rs:886-890`) meaning nl (H D2). The extension's reader must handle the object form too.
* Logs stay as they are (H D10); machine markers stay byte-identical (H D11); on-disk XDG folder names are never renamed (H D12).
* **Robin:**
  * "Win11 Shell" against "Win11 voor GNOME" in about 50 Settings strings. Decide before round 3, to avoid translating twice.
  * The `metadata.json` name and description (they cannot be localised).
  * The NucBox's language and H clean-up (F20).
  * The live deploy and the unknown live `win11-panel` (F22).

## F. Memory and the heavy lock

**F32 (Critical): earlyoom here protects exactly the processes the harness starts.**
* **Evidence.** The running earlyoom is pid 1924: `-m 8 -s 10 -r 3600 --sort-by-rss --avoid ^(gnome-shell|Xwayland|gdm.*|systemd.*|dbus.*|pipewire.*|wireplumber|gnome-session.*)$ --ignore ^(win11-panel|win11-app|WebKitWebProces|WebKitNetworkPr|WebKitGPUProces)$`.
  * Test copies of `win11-app`, `win11-panel` and every WebKit child are **never** killed. The headless test `gnome-shell` is avoided.
  * Victims are picked by RSS, so `choom`/`oom_score_adj` does not steer earlyoom.
  * earlyoom acts only when both MemAvailable < 8 % (≈ 1.26 GB of 15.7 GB) **and** free swap < 10 %. Swap is already 51 % used (5.9 of 12 GB free), so the machine swaps for a long time first, which Robin feels in his live session.
  * When earlyoom finally acts, it kills Robin's largest unprotected process: a browser, an editor, a desktop app, or our own rustc or Chrome.
* **Change:**
  * Every GUI harness runs a watchdog that samples MemAvailable every second and kills its **own** process group below 1.5 GB, then writes the event to its log.
  * Cap the concurrent WebKit windows per run (for example 3).
  * Optional, coordinator's call because it touches Robin's live user manager: run the harness inside `systemd-run --user --scope -p MemoryMax=1.5G -- dbus-run-session …`. This is a transient scope, removed on exit.

**F33 (Medium): the memory check happens before the lock.**
* **Evidence.** `tests/rust-sandbox/run-tests.sh:27-31` checks MemAvailable, then waits up to an hour in `flock -w 3600` at `:96`. The check can be an hour stale.
* **Change.** One shared wrapper, `tools/heavy.sh`, used by every agent:
  1. take the lock;
  2. **inside it**, check MemAvailable ≥ 2 GB (2.5 GB for gnome-shell runs and the release LTO link) and free disk ≥ 5 GB;
  3. run under `nice -n 10 ionice -c3`, with `/usr/bin/time -v` into `tests/logs/heavy-<agent>.time`;
  4. on a lock timeout, stop and report; never run without the lock.

**F34 (Medium): `build.sh` is not offline and defaults to 4 jobs.**
* **Evidence.** `scripts/build.sh:13`: `cargo build --release --locked -j "${WIN11_BUILD_JOBS:-4}"`. It has no `--offline`, and gnome-map §5 recommends using it.
* **Change:**
  * Always set `CARGO_NET_OFFLINE=true WIN11_BUILD_JOBS=2`, or call cargo directly with `--offline --locked -j2`.
  * The first release build (thin LTO, 12 binaries) records its peak RSS. If one rustc goes over 2 GB, use `-j1`.
  * Prefer `cargo check` during conversion work; run `cargo test` at the end of a step.

**F35 (Medium): disk.**
* **Evidence.** 23 GB free on `/` (90 % used); `target/` is 954 MB today.
* **Expected growth:**
  * a release target: a few GB (estimate);
  * 4 worktree checkouts: each carries 81 MB of icons plus 18k symlinks;
  * `tests/build/3b898e6-debug` (≈ 340 MB);
  * package staging.
* **Change.** The `heavy.sh` disk check, one shared `CARGO_TARGET_DIR`, and remove worktrees after the merge. "About 3.5-4 GB free" in `plan.md` is stale: 6.6 GB at 13:10. Do not hard-code it.

## G. Smaller points

* **F36 (Low): `tests/gnome/` is used by more than one step.** GE0c owns the harness core. Later steps add scenario files only (`tests/gnome/scenarios/<step>-*.py`), so the harness needs a scenario plug-in interface from the start. Codex's `W/desktop-language-test/checks.py` can be ported as the first scenario.
* **F37 (Low): H is frozen, but not recorded.** H has not changed since 12:28 (`english-option-design.md`; `H/ui` since 09:42). GE0a should write `notes/claude/h-snapshot.sha256` for `H/ui`, `H/i18n`, `H/tools/i18n` and `H/tests/i18n`. GE1a re-verifies `proposed.tsv` hashes against H and G before applying.
* **F38 (Low): the app catalogue bug on a Dutch system** (rust-apps item 7, `apps/catalog.rs:300-308`). In English on a Dutch-locale machine (the NucBox), app names fall back to the Dutch GIO name. Assign it to R2-D.
* **F39 (Low): keep `tools/i18n/check.py` thin.** GE1b and GE1c both need new rules. Put the rules in `check_ui.py`, `check_rust.py` and `check_pairs.py`, with `check.py` only dispatching, and set that layout up in GE0a. Allowlists also go per layer: `allow-ui.txt`, `allow-host.txt`, `allow-ext.txt`. `BASELINE-CHANGES` becomes `.ui`, `.rust` and `.ext`.

## H. Proposed file ownership

### Round 1 (after the changes above)

Each path has exactly one writer. "Contract" means the format is fixed in `notes/claude/contracts.md` before the round
starts (F6).

| Path | Writer | Notes |
|---|---|---|
| `vendor/ui/**` (all of it) | **GE1a** | The 43 `proposed/` files; `alles/main.js:420-423` → key; `explorer/main.js:368`; `explorer/tabtransfer.js`; `explorer/picker.js:49,88`; `settings/pages/power.js:78`; `apps/appframe.js` (native `setTitle`, and `onLangChange` → `setTitle` in taskmgr, alles, disks and explorer); the D-G2 UI side if Robin agrees (`settings/pages/clipboard.js`, `display.js`); en overflow fixes (F10) |
| `vendor/ui-sha256.json`, `vendor/ui-patches.json` | **GE1a** | Via the new `tools/ui-manifest.py` |
| `tests/i18n/{surfaces.json,BASELINE-CHANGES.ui,allow-ui.txt,overlay/**,dict-nl-*.tsv}`, `tools/i18n/check_ui.py` | **GE1a** | GE0a created them; GE1a extends them |
| `host/src/i18n.rs` (new), `host/i18n/*.tsv` (all stems, created empty except `core`) | **GE1b** | The include list is complete (F8) |
| `host/src/lib.rs` | **GE1b** | Only `mod i18n;`, `mod desktop_entries;` and the panel hook for F3 |
| `host/src/files/mod.rs` | **GE1b** | The `ApiError` block (`:28-50`) only; the 26 literals wait for R2-C |
| `host/src/runtime.rs`, `host/src/services/mod.rs` | **GE1b** | The `core` pilot (envelopes, `unknown()`, `:87`, `:116`); no `X-Shell-Lang` (F9) |
| `host/src/services/notifications.rs` | **GE1b** | `:361` `""`, plus mapping stored "Melding" on load |
| `host/src/window.rs` | **GE1b** | nl/en title table at creation; the existing `setTitle` handler |
| `host/src/desktop_entries.rs` (new), `host/src/desktop-entries.json` (new) | **GE1b** | F3; `install.py` reads the JSON |
| `tools/i18n/{extract_rs.py,check_rust.py}`, `tests/i18n/{BASELINE-CHANGES.rust,allow-host.txt}` | **GE1b** | F8, F12, F13, F15 |
| `extension/*.js` | **GE1c** | `lang()` and monitor, labels, clock, Ctrl+Alt+Tab, `'Everything'` per D-G5, bilingual failure texts (F4); no new files unless `install.py:104` is updated (F18) |
| `extension/metadata.json`, schema | nobody in round 1 | Version bump in round 7; name per Robin |
| `scripts/**` (`install.py`, `uninstall.py`, `icon_bundle.py`, `file_portal.py`, `build.sh`, `package.py`, `verify-ui.py`, `*.sh`) | **GE1c** | `--lang`, messages, entries from `desktop-entries.json` in the current language (F19), prefer `target/release` or require all 12 `BINS`, preflight, H-language migration (F20) |
| `tests/installer/**`, `tests/gnome/scenarios/ge1c-*`, `tools/i18n/check_pairs.py`, `tests/i18n/BASELINE-CHANGES.ext` | **GE1c** | |
| `tests/gnome/**` (core), `tests/build/**`, `tools/heavy.sh` | GE0c / GE0b (frozen in round 1) | Round-1 steps add scenario files only |
| `Cargo.toml`, `Cargo.lock`, `host/src/api-methods.json`, `host/src/events.rs`, `host/src/settings*.rs`, the rest of `host/src/**` | **nobody** | Read-only in round 1. No new API method is needed in round 1 |
| `notes/claude/plan.md`, `contracts.md` | coordinator | Each agent writes only `notes/claude/<step>.md` |

**Overlaps the current plan has, and where this table puts them:**

| Overlap | Status |
|---|---|
| Desktop rewrite in host versus GE1c | Moved to GE1b (Rust) plus GE1c (`install.py`) via one JSON file (F3) |
| Error mapping: `shell.js` (GE1a) versus `gnome.rs` versus the extension (GE1c) | Extension only (F4) |
| `verify-ui.py` (GE1a generator versus GE1c messages) | GE1c only; the generator is a new tool (F5) |
| `lib.rs` (GE1b `mod i18n` versus a host-side desktop rewrite) | GE1b only |
| `window.rs` ↔ `appframe.js`; `runtime.rs` ↔ `bridge.js`/`hostcall.js`; `notifications.rs` ↔ `toasts.js`/`notifcenter.js` | Different files; contract (F6) |
| `BASELINE-CHANGES`, `check.py`, allowlists | Split per layer (F39) |
| `target/` | GE1b only; GE1c uses `tests/build/3b898e6-debug` (F7) |

### Round 2 (four agents, each in its own worktree; F2)

| Agent | Owns (exclusive) | TSV stems | ≈ sites |
|---|---|---|---|
| **R2-A** system and settings | `services/system.rs`, `services/system/**` (audio, bluetooth, agent, network incl. the Wi-Fi pairs, energy, hardware, power, info), `services/display.rs`, `services/media.rs`, `services/notifications.rs` (the rest), `services/shell.rs`, `services/shell/**`, `services/tray.rs` + `tray/**`, `settings.rs` (incl. the `:136` export name), `settings_runtime.rs`, `settings_effects.rs` (markers `keep`) | `system`, `display`, `media`, `notifications`, `shell`, `clipboard`, `filemanager1`, `tray`, `settings` | ≈ 87 (+ D-G2 Rust: `power.rs:154`, `display.rs:86`) |
| **R2-B** monitoring, storage, disks, index | `services/sysmon.rs` + `sysmon/**` (incl. `_FRIENDLY`, `unit_friendly`, D-G9), `services/storage.rs` + `storage/**`, `services/disks.rs` + `disks/**` (language-keyed caches), `services/index.rs` + `index/**`, `bin/index_worker.rs` (`index.status` re-send, root errors as keys) | `sysmon`, `storage`, `disks`, `index` | ≈ 112 |
| **R2-C** files | `files/**` except the `ApiError` block of `files/mod.rs`; `files/types.json`, `files/launcher-names.json` (`io_error`, `validate_name` and `detail_error` are used only inside `files/`, checked by grep) | `files`, `files_launch` | ≈ 133 |
| **R2-D** apps and native glue | `apps/**` (incl. the F38 fix and the `launcher-names` dedupe), `picker.rs`, `transfers.rs`, `process_scope.rs`, `gnome.rs` (incl. `:25`), `wallpaper.rs`, `launch.rs`, `bin/launch.rs`, `backend.rs` (internal only, optional) | `apps`, `picker`, `transfers`, `launch`, `gnome`, `wallpaper` | ≈ 113 |
| **frozen in round 2** | `i18n.rs`, `lib.rs`, `window.rs`, `runtime.rs`, `services/mod.rs`, the `ApiError` block, `api-methods.json`, `events.rs`, `Cargo.*`, `services/software*` (already bilingual pairs: checker-marked, no edits), `vendor/**`, `extension/**`, `scripts/**`, `host/i18n/core.tsv` | – | – |

**Shared round-2 rules:**
* Only the converter writes TSV rows.
* Each agent writes its own allowlist and baseline-change file: `tests/i18n/allow-host-r2x.txt` and `BASELINE-CHANGES.rust-r2x`. The coordinator merges them.
* Done-when for each agent:
  * the drift check (F12) is clean for its paths;
  * the oracles are green (F29);
  * the ratchet is clean;
  * `cargo test` is green in the sandbox;
  * there are en render tests for its stems.

## I. Suggested order (delta to plan.md)

1. **Now:**
   * one batched Robin question on D-G1 (a/b), D-G2, D-G5 and branding;
   * `contracts.md`;
   * gitignore the sandbox state, `.rt/` and `.wt/`;
   * `tools/heavy.sh` with the watchdog.
2. **Round 0, as planned, plus:**
   * F23-F25 in GE0a and GE0c;
   * GE0c waits for GE0b's build, and the baseline binaries are kept;
   * the overflow audit (F10);
   * the dictionary snapshot.
3. **Round 1:** GE1a, GE1b and GE1c per the table. GE1b ends with the round-2 gate (F8).
4. **Round 2:** four worktrees, then the coordinator merges and runs a full drift check and `cargo test`.
5. **Round 3:** as planned, minus widths (already done); plus the pixel checks on the mock WebKitGTK page (F26).
6. **Rounds 4-6:** Windows Update. D-G14 can run in parallel with round 4 (F11). Decide the root step (F21) in round 4.
7. **Round 7:** packaging (F21), then the Robin-only live deploy with one re-login (F22), then the NucBox steps after the H inventory (F20).

## Not checked

* I did not run any scanner, build, test or harness. The socket-path lengths are computed from the plan's suggested layout, not measured on a live socket.
* `~/.config/win11shell`, `~/.local/share/applications` and `mimeapps.list` on this PC were not read (they are off limits). F20's "very likely on this PC" rests on port-path F1 (`~/.local/share/win11shell/ui` exists) and was not verified further.
* The earlyoom flag semantics (`--sort-by-rss` ignores `oom_score_adj`; it kills only when both thresholds are crossed) are as documented upstream, from memory. I did not test them here.
* The release-build memory peak and size are not measured; F34 and F35 are estimates.
* I did not review GE0a's and GE0b's in-progress files beyond `tests/rust-sandbox/run-tests.sh` lines 20-110 and `session-bus.conf`, which were read as evidence for F23 and F33.
* I did not re-verify per-file literal counts, the merge-trial output, or H's English quality.
