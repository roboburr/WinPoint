# GE1c review: extension and installer (2026-11-05)

Adversarial review of GE1c (`notes/claude/round1/ge1c.md`), with GE1h's scenario harness. G = this repo, branch
`claude/gnome-english-update`. Nothing was committed. I edited one file, `tests/gnome/scenarios/ge1c-extension.py`.
It is in GE1c's namespace and the task gave it to the review; I rewrote it and kept all of the owner's checks. I also
wrote this note. Its run output replaced `tests/gnome/scenarios/ge1c-extension.out/`. No product file was touched. The
GE1c files kept their sha256 from the start of the review to the end (`sha256sum -c`; only my scenario differs).

**Verdict: no blocker.** Dutch is unchanged apart from the listed rows. Every new script and test is
sandboxed or read-only. The extension follows the language live on all three write paths and on the C1 edge values.
There is one **major** finding: 12 English failure texts are not en-GB and do not use Windows 11 wording. There are
9 minor findings and nits, plus one cross-layer C1 defect in the UI that belongs to GE1a.

## 1. What I ran (from G)

| Command | Result |
|---|---|
| `python3 -B tools/i18n/check_pairs.py --warnings` | 0 errors, 0 warnings, 143 pairs in 11 tables |
| `python3 -B tools/i18n/check.py` (dispatcher) | 0 errors, 54 warnings (all UI glossary, GE1a); the pairs layer is included |
| `W11_INSTALLER_REAL=0 tests/installer/run.sh test_extension_text` (in private.sh) | 7 tests OK in 1.2 s |
| `tests/installer/run.sh` (heavy.sh `--min-mb 1024` + private.sh + bwrap) | **35 tests OK in 229.5 s** (3 min 50 s wall). Real nl update: 140,216 mutating syscall paths, 0 outside the case dir. Real en first install: 140,223 paths, 0 outside |
| Node dump of `extension/` in nl and en, side by side (fake world, my own config dirs) | 56 visible rows; read every one by hand (section 3) |
| Glossary over all 143 pairs (scratch script that reuses `check_pairs` and `tools/i18n/glossary.tsv`) | 9 hits: 3 are real (M1); 6 are intended (Dutch UI path in an nl-language line, "achtergrond" = backdrop) |
| `install.preflight_runtime()` on `tests/build/3b898e6-debug` (inside private.sh) | ok: 14 DT_NEEDED and `libarchive.so.13` all resolved on this machine |
| Interrupted first install, then rerun (inside private.sh + `sandbox.Case`, en_GB locale) | demonstrates m1 |
| `tools/h-remnants.sh` on a G-only fake HOME (scratch, `env -i`, `W11_REMNANTS_SYSROOT`) with H-style `win11shell caption-buttons` blocks in `gtk.css` | reports them (exit 1); G itself never writes those blocks (`settings_runtime.rs:53`, `layer: false`), so this is correct |
| `tools/heavy.sh --min-mb 2560 --tag ge1c-review -- python3 -B tests/gnome/harness.py launch run --lang nl --build-dir tests/build/3b898e6-debug --scenario tests/gnome/scenarios/ge1c-extension.py` | run 2 (final): **exit 0, 64 checks, 63 pass, 1 failure, marked known** (`C6-runtime`); 120.1 s; peak PSS 934 MiB; min MemAvailable 4,423 MiB; install 87,844 paths, 0 outside `run/`, 12/12 binaries and 9/9 extension files identical; no leftover processes |

Run 1 of the scenario failed 2 checks because of a bug in my probe: it imported `/js/core/bridge.js`, but the page
lives under `/ui/`. I fixed the probe to use `document.baseURI`. Run 1's other 61 results match run 2.

UI dumps, the dictionary diff, drift_rust and the Rust oracles cover files that GE1c does not touch, so I did not
rerun them for this review. The installer output, the extension dump and the private-shell scenario are GE1c's Dutch
baselines, and I ran all three.

## 2. Lens 1: Dutch unchanged

* **Installer.** For Robin's update case (an nl `settings.json`), the new installer and the 3b898e6 one were run in the
  same case directory. They give byte-identical stdout, stderr and installed trees, both on the fixture and on the real
  tree (a path with spaces, `%` and `$`). The same holds for:
  * the D13 update without settings on an English locale;
  * the single-missing-binary message;
  * `uninstall --files-only`.

  I reran all of these myself (35/35). A first nl install differs only by the banner line and
  `config/win11-gnome/{settings.json,.settings.lock}`; both are listed in `BASELINE-CHANGES.ext`.
* **Extension.** The nl dump differs from 3b898e6's in exactly the 38 `key:ext.*` rows, and the codes are unchanged.
  I checked every one of those rows against the decisions:
  * **29 failure texts and 2 notification failures.** These were English in Dutch mode at 3b898e6. Turning them
    into Dutch is planned work (plan GE1c, F4), not D-G1a/b or D-G2. The Dutch is natural and matches the UI's
    vocabulary ("app", "Verkenner", "geminimaliseerd").
  * **6 launch-context names.** They now match `desktop-entries.json` and D-G5 ("Alles"). They are only used when a
    `.desktop` file is missing, so they are practically never shown.
  * **`appName ""`** (C5). The UI turns it into "Melding".
* **Live Dutch checks in the private shell all pass:**
  * the accessible names "Start", "Zoeken", "Taakweergave" and "Netwerk, geluid en snelle instellingen";
  * the Ctrl+Alt+Tab group "Win11-taakbalk";
  * the clock `HH:MM\n05-11-2026`;
  * the notification "Win11 voor GNOME / Deze app is nog niet geïnstalleerd.";
  * the names "Win11 Verkenner" and "Win11 Taakbeheer" (install.py).
* **For the final report.** Robin should see the Dutch rows as a group: "29 failure toasts are now Dutch in Dutch
  mode". They are not hidden, but nobody has shown them to him yet.

## 3. Lens 2: English quality

The labels, accessible names and group are correct:
* "Search", "Task view", "Network, sound and quick settings";
* "Win11 taskbar";
* the en-GB clock `05/11/2026`;
* "This app isn’t installed yet.", "Task Manager isn’t installed yet.";
* the desktop names (Win11 File Explorer, Task Manager, Disk Management, Default apps, App Installer, "Alles").

The installer and h-remnants messages read naturally, and the bilingual banner is a good idea ("In het Nederlands:
kies Nederlands in Settings › …", with the UI path in the language the UI will show). No Dutch is left:
* the en dump has 0 Dutch-word hits;
* the scenario's en checks all pass.

**Failure texts: see M1.**

## 4. Lens 3: robustness, contracts and live safety

**Live session.** No script or test can touch Robin's session:
* The installer tests run in bwrap: the root is read-only, the real home is a tmpfs with only G bound back
  read-only, and there is no network and its own PID namespace. They also run inside private.sh, and the heavy ones
  under heavy.sh.
* The Node test reads only its own config dirs.
* `h-remnants.sh` is read-only: it uses `find`, `du`, `grep`, `pgrep -a` and `python3 -I` reads, and it signals nothing.
* My scenario writes only in `tests/gnome/run/`.
* The owner ran `gnome-shell --version` once outside a sandbox (m11).

**Live switching in the real private shell (3b898e6 binaries, worktree extension).** The latency is from the file's
mtime to the extension's re-label, measured by a probe wrapped around `LanguageWatch._onChange`:

| Path | Latency |
|---|---|
| Dropdown (Settings writes by rename) | 111 ms |
| Rename | 100 ms |
| In-place rewrite | 100 ms |

That is the 100 ms throttle; there was exactly one re-label per switch. With the panel stopped, the fallback bar
appeared after 0.12 s and was English at once. Switching to nl and back while the bar was visible worked, with
screenshots `02-en-fallback-bar.png` and `03-nl-fallback-bar.png`.

**C1 edge values, live:**
* `"EN"`, the object form and a moved-away `settings.json` each gave nl in about 0.1 s;
* moving the file back gave en again;
* the monitor is `WATCH_MOVES` (flags 8), on the config folder, with no parent monitor while the folder exists.

**Failures in the language of the moment:**
* The in-process `Call` gives the right nl and en texts.
* End to end, panel page → win11-panel → D-Bus → extension → error → the page's own `Toast`, the toast showed
  "Het gevraagde app-venster bestaat niet meer." in nl and "The requested application window no longer exists." in
  en (`nl-failure-toast.png`, `en-dropdown-failure-toast.png`).
* The extension's `Main.notify` notification is mirrored as a Win11 toast within 0.1 s in both languages
  (`*-missing-toast.png`).

**C6 runtime path, tested on its own.** The scenario rewrote `Name=` of the Explorer entry the way win11-panel will
(temp file + rename). The pinned button followed through the new `installed-changed` handler: "Win11 File Explorer"
after 5.63 s, and back to "Win11 Verkenner" after 5.83 s. About 5 s of that is GNOME's (GIO/AppSystem) latency, not
the extension's; see m9. The only failed check is the known one, because the 3b898e6 panel has no rewriter (GE1b).

**Contracts:**
* **C1:** the extension reads only `GLib.get_user_config_dir()/win11-gnome/settings.json` and creates nothing. The
  Node test shows `meta.reads` is only that file.
* **C3:** inline tables everywhere; codes unchanged.
* **C5:** ok.
* **C6:** install-time names are in the stored language; reinstall after a switch keeps English (test).
* **C7:** listing minus `schemas/`; harness `extensionIdentical` 9/9.
* **C8:** all heavy runs went through heavy.sh.

**Widths, measured only:** the fallback search button is 178 px in both languages; the label is 58 px nl and 56 px en;
it never goes compact at 1440 px. This matches the owner's numbers.

## 5. Findings

### Blocker: none

### Major

**M1. 12 English failure texts are not en-GB and do not use Windows 11 wording, and they no longer match their new Dutch.**
* **Where:** `extension/panelBridge.js` `TEXT`, `desktopBridge.js`, `frostedBridge.js`, `notificationsBridge.js`.
* **Evidence:**
  * The owner kept 3b898e6's developer English while writing new, user-facing Dutch. The UI shows these texts in toasts
    (`shell.js:459-461`, plan-critique F4); the scenario screenshot `en-dropdown-failure-toast.png` shows one.
  * Glossary and D-G15 (en-GB) problems:
    * `cantMinimize`/`cantMaximize` say "minimized"/"maximized"; the glossary and `appframe.js` use
      "Minimise"/"Maximise".
    * `panelMissing` ("De taakbalk …") says "The native panel is unavailable."; the glossary has Taakbalk → Taskbar.
    * `explorerGone` ("Verkenner-venster") says "Explorer window"; the glossary has Verkenner → File Explorer.
  * Jargon a user never sees in Windows 11:
    * "native host", "native method", "native application window is not mapped", "has not painted its first frame",
      "normal window frame";
    * "application" where Windows 11 and the extension's own `appMissing` say "app".
  * Style: the extension itself uses "isn’t", but the failures say "is not".
* **Fix:** change only the `en` values; nl, the BASELINE rows and the codes stay as they are.

  | Key | New en |
  |---|---|
  | `startMissing` | Win11 Start isn’t installed. |
  | `sessionApps` | Apps can only be opened in the unlocked user session. |
  | `appMissing` | The requested app isn’t installed. |
  | `panelMissing` | The taskbar isn’t available. |
  | `badHost` | Invalid app process. |
  | `hostUnmapped` | The app’s window isn’t shown yet. |
  | `hostUnpainted` | The app hasn’t drawn its window yet. |
  | `badGeometry` | Invalid saved window position. |
  | `waitFrame` | Waiting for the restored window. |
  | `windowGone` | The requested app window no longer exists. |
  | `explorerGone` | The original File Explorer window is no longer available. |
  | `cantMinimize` / `cantMaximize` / `cantClose` | This window can’t be minimised. / maximised. / closed. |
  | `unknownMethod` | Unsupported GNOME function: {method} |
  | `badSnap` | Unknown snap position for the window. |
  | `offline` | The GNOME integration is turned off. |
  | `badPid` | A valid desktop process ID is required. |
  | `badIdentity` | Invalid identity for the window backdrop. |
  | `identityMismatch` | The backdrop identity doesn’t match the window. |
  | notifications `offline` | The notification bridge is turned off. |

  When it lands:
  * update `FAILURE['en']` in `tests/gnome/scenarios/ge1c-extension.py`;
  * update any en expectations in `tests/installer/ext/dump.mjs` or `test_extension_text.py`;
  * add a glossary rule to `check_pairs.py` (m2) so it cannot come back.

### Minor

**m1. An interrupted first install loses the language choice.**
* **Cause:** `store_language()` runs last in `install.py:main`. When anything fails after the payload rename
  (`glib-compile-schemas`, `update-desktop-database`, a portal or icon step), the rerun sees the payload. It then takes
  the D13 path: nl, nothing stored, no banner.
* **Evidence:** in a sandboxed case with an en_GB locale and a `glib-compile-schemas` that exits 1, run 1 exits 1 with
  the payload present and no `settings.json`. Run 2 exits 0 with no `settings.json`, first line "Geïnstalleerd in …"
  and `Name=Win11 Instellingen`. A clean first install on the same locale prints "Language: English…" and stores en.
* **Fix:** call `store_language()` right after the preflight and `check_config_dir()`, before the stage is created. It
  never overwrites a file, so storing early is harmless. Add this test: a failing stub, then a rerun, expecting en.

**m2. `check_pairs.py` has no glossary rule,** although `check_ui.py` has one, so M1 passed the checker.
* **Fix:** apply `tools/i18n/glossary.tsv` to the pairs, as warnings until the round-3 lock-in. Allow these on purpose:
  * the Dutch UI path inside the nl-language lines `langZone`/`langFormats`;
  * achtergrond → backdrop.

**m3. The installer and uninstaller now silently delete unknown top-level files in the extension folder.**
* **Where:** `install_extension()` and `uninstall.py`, in
  `~/.local/share/gnome-shell/extensions/win11-gnome@win11shell.local/`.
* **Risk:** on Robin's live folder, which only Codex's deploys wrote and nobody has inspected, extra files would be
  gone without a trace. The round-7 backup list from F22 names `~/.local/bin/win11-*` and `~/.local/share/win11-gnome`,
  but not that folder.
* **Fix:** print each removed name (bilingual), and add the extension folder to the round-7 backup step.

**m4. The "debounce" is a throttle.**
* **Behaviour:** the first event schedules one read after 100 ms, and the events after it are dropped until then. With
  a non-atomic writer that takes longer than 100 ms, the read can see a truncated file. That flips to nl for a moment
  (C1: broken JSON means nl) and re-labels twice.
* **Scope:** the store writes atomically, and the in-place rewrite of the harness passed, so only third-party writers
  are affected.
* **Fix (optional):** restart the timer on every event, or re-read once on a parse error before applying nl.

**m5. Each language switch adds one more `destroy` handler to the bar.** GNOME's
`CtrlAltTabManager.addGroup` connects `root.connect('destroy', …)` and never disconnects it (`ctrlAltTab.js:34`,
read from `libshell-18.so`). After N switches the bar holds N closures. This is harmless (`removeGroup` is
idempotent), and there is no visible order change, because items sort by X. **Fix:** none needed, or note it in a
comment.

**m6. The en variants of `T.langZone` and `T.langFormats` are unreachable.** Those lines are printed only when the
locale chose nl, so messages are then in nl. **Fix:** keep them for the checker's parity, or drop them; add a comment
either way. A nit in the same table: `badLang` quotes with ‘…’ in nl and “…” in en.

**m7. `scripts/build.sh` still runs `cargo build --release --locked -j ${WIN11_BUILD_JOBS:-4}` without `--offline`**
(F34). GE1c owns the file, but this is not in its round-1 deliverables. Fix it in round 7, or now with
`--offline` and a default of `-j2`.

**m8. Main.notify banners carry GNOME's "System" source header in GNOME's locale in both Win11 languages**
(`nl-missing-toast.png`). This is pre-existing and D-G11 territory. Using the extension's own `MessageTray.Source`
titled with the product name would change nl output, so only Robin should decide it. Informational.

**m9. GNOME reflects a C6 `Name=` rewrite after about 5 s** (5.63 s / 5.83 s from the rename to the pinned
button). The extension reacts as soon as `installed-changed` fires, so the delay sits in GIO's desktop-dir monitor and
Shell.AppSystem. For GE1b and round 3: GNOME's own surfaces (overview, pinned button, Alt+Tab names) lag the panel by
about 5 s. A check written for GE1b's `--build-dir target/debug` rerun needs a wait of 10 s or more.

**m10. The owner's scenario covered neither the dropdown path nor a toast.** Fixed in the review scenario, which now
adds:
* the dropdown, rename and rewrite paths;
* the end-to-end failure toast;
* the Main.notify toast mirror;
* the C1 edge values;
* the simulated C6 rewrite;
* the re-label latency probe;
* nl and en on the visible fallback bar.

**m11. Rule deviation, already reported by the owner:** `gnome-shell --version` was run outside the sandbox. It is
harmless, because it only prints the version, but it breaks the letter of the hard rule. No action, except the
coordinator noting it.

### Cross-layer (not GE1c's files): the UI does not apply C1 to unknown values (GE1a, **major for GE1a**)

* **Where:** `vendor/ui/js/core/theme.js:319-321`. The code is `const language = forcedLang || s?.language || 'nl';
  setLang(language); html.lang = language;`. `core/i18n.js` `setLang()` ignores any value other than nl and en, so the
  UI **keeps its previous language**, and `<html lang>` gets the raw value.
* **Evidence from the scenario:**
  * After `"language": "EN"`, the extension (and GE1b's Rust host) is nl, but the panel stays English, with
    `lang="EN"` and the clock "05/11/2026".
  * With the object form, `lang="[object Object]"`.
* **Fix:** `const language = forcedLang || (s?.language === 'en' ? 'en' : 'nl');`, so the theme cache line also
  agrees. This is for GE1a or the cross-layer check.

## 6. Owner's decisions: my recommendations

1. **`readSettings()` H fallback.** Keep it in round 1. Dropping it changes Dutch behaviour (pins, alignment, search,
   seconds) on a machine that has H's file but no G `settings.json`, which may be this PC. Decide after
   `tools/h-remnants.sh` has run here and on the NucBox, then drop it with a behaviour row in `BASELINE-CHANGES.ext`.
2. **"Win11 for GNOME".** This is a fair reading of D-G16. Ask Robin together with the `metadata.json` name (F31), because
   GNOME's extension list keeps "Win11 voor GNOME — Bureaublad en taakbalk".
3. **`--lang` ignored on a stored choice.** Keep it (C6 as written). The note already tells the user where to change the
   language.
4. **A first install stores nl too.** Accept it. Be aware that a complete defaults document pins every default value at
   install time: a later change to an existing default will not reach users who never changed that setting. New keys
   still merge.
5. **Hard requirements.** Accept them. On this PC the preflight passes for the 12 baseline binaries.
6. **Commit `ge1c-extension.out/`.** Commit it: 748 KB, 10 PNGs, `results.json` and logs. It is now the review run.

## 7. Left

* M1 and m1-m3 are for the GE1c fix step. After the fix, rerun `tests/installer/run.sh` and the scenario.
* After GE1b's panel rewriter lands, rerun the scenario with `--build-dir target/debug`, raise the C6 wait to at least
  10 s (m9), and turn `known='C6-runtime'` into a hard check.
* The cross-layer C1 defect in `theme.js` (GE1a).
