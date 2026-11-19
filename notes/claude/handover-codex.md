# Handover to Codex (17-11-2026, about 02:15)

Claude stopped here at Robin's request ("schrijf een handover, codex gaat verder"). Every background job is stopped,
and every unfinished change is committed as a labelled WIP commit. Nothing is pushed. This file says where things
are, what is done, what is not, and how to continue.

## 1. What Robin asked

* **Settings parity (D-G77, 16-11):** "alle settings van de originele GNOME functies moeten erin komen, maar in de
  style van windows settings app. kijk vooral af ... bijvoorbeeld muis settings mist". Every GNOME Settings 50
  function goes into WinPoint's Settings app (native/apps/settings, GTK 4 + Rust), in the structure, layout and
  wording of Windows 11 Settings (nl-NL and en-GB). Robin asked Claude to keep going until the work was done and
  then give one short Dutch report.
* **Earlier work, all done:**
  * D-G75: 0.9.1, the easier install. Committed, NOT released.
  * D-G76: a taskbar on every display. Merged as af8eebdc.
  * Both are on branch claude/gnome-english-update (main checkout, tip e56ab407 + this note).

## 2. Rules that hold on this machine (Robin's standing rules)

* **Never act on Robin's live session.**
  * No D-Bus calls, gsettings, dconf, gdbus or busctl writes.
  * No signals or kills to his gnome-shell, win11-shell, win11-panel or ~/.local/bin programs.
  * No installs into ~/.local.
  * All tests run in the sandboxes: tests/rust-sandbox, tests/dbus-fakes and the private headless GNOME of
    tests/gnome/harness.py.
* **Pushing, a release, deleting a tag, sudo and passwords are Robin's.** Never push without his explicit go. The
  repo (github.com/roboburr/WinPoint) is public: scan new content for personal data before any push. Never push the
  local tree before the 0.9.1 release exists, because README's install line names the unreleased 0.9.1.
* **Builds and heavy jobs only through tools/heavy.sh.** It is a machine-wide lock and needs 5 GB free disk. The root
  disk has about 5.8 GB free, so write nothing big to / or /home.
* **Commits:** explicit paths. Claude committed as `-c user.name=Claude -c user.email=noreply@anthropic.com`.
* **Never run tests/update/run.sh --self-test during a harness run.** It leaves a socket in the checkout.
* **Run time stays GTK 4 + Rust.** No WebKit, no GTK 3, no Python at run time (D-G72/73).
  tools/check-no-webkit.py is the acceptance check.
* **UI strings:** every one is an nl + en pair in Windows 11's own wording, with no em-dashes. Reports to Robin are
  in Dutch, short, without em-dashes and without a closing question.
* **GNOME and Ubuntu sessions:** never write ubuntu-dock, DING or tiling-assistant schemas, or enabled-extensions,
  in a writing mode. Nothing may change the normal sessions beyond what GNOME Settings itself writes.
* **Do not work on Klonkt** (another project on this machine).

## 3. Where everything is

| Place | What |
|---|---|
| `/home/robin/Documents/Codex/2026-11-01/chat/outputs/win11-gnome` | Main checkout (ext4), branch claude/gnome-english-update |
| `notes/claude/settings-parity.md` | **The contract.** §1 the 69 amendments, §2 the page tree, §3 frozen shared files, §4.x ownership per batch, §5 host API, §6 foundation API, §7 tests, §8 integration notes |
| `notes/claude/settings-parity/` | Sources: plan.json (every page's controls with GNOME function, keys, writes, testing), critique.md, gnome-a/b.json, windows.json, winpoint.json |
| `/run/media/robin/22TB/win11-gnome-wt/{a..f}` | One git worktree per batch (NTFS) |
| `/run/media/robin/22TB/win11-gnome-build/wt-<x>/{host-target,native-target}` | Each worktree's own warm cargo targets |
| `/run/media/robin/22TB/win11-gnome-wt/helpers/` | Build, test and harness helpers for the worktrees, the step I work list, all batch results |

**NTFS (fuseblk):** every file reads 0755. ALWAYS pass `git -c core.fileMode=false` in the worktrees, and never
change the git config. The harness refuses to run on NTFS, so `helpers/harness.sh` copies the worktree to
`/tmp/win11-gnome-h-<x>` (tmpfs), restores the exec bits from git's index, runs the harness there, copies the result
dirs back and deletes the copy.

**Helpers** (`<x>` is the worktree letter a-f; logs in `helpers/logs-<x>/`):
* `helpers/build.sh <x> [host|native|all]`: release builds into the worktree's own targets, then refreshes
  `tests/build/native-release` (19 binaries + SHA256SUMS).
* `helpers/host-tests.sh <x> --lib [-- <filter>]`: host unit tests in tests/rust-sandbox.
* `helpers/native-tests.sh <x>`: `cargo test --release` for native/.
* `helpers/checks.sh <x>`: rustfmt, no-webkit, the i18n checks, test_native.py, settings-pages, installer, release
  check.
* `helpers/harness.sh <x> -- launch run --lang nl|en --source worktree --scenario tests/gnome/scenarios/<s>.py [--out tests/gnome/scenarios/<s>.en.out] [--strace <file>]`
* They retry heavy.sh's exit 75 (lock busy) every 120 s. Builds use `--release --offline --locked -j 2`, and no new
  crates.
* Cargo homes: host `/run/media/robin/extra/win11-gnome-build/cargo-home`, native
  `/run/media/robin/22TB/win11-gnome-build/cargo-home`.
* Main-checkout builds use the targets `/run/media/robin/extra/win11-gnome-build/target` (host) and
  `/run/media/robin/22TB/win11-gnome-build/target` (native, `--manifest-path native/Cargo.toml`).

## 4. State of the work

Base of all batch branches: af8eebdc (B0 foundation + multi-monitor merge).

| Worktree | Branch | Tip | State |
|---|---|---|---|
| a | claude/settings-a | ac98ef65 | Batch A (input & accessibility, 18 pages): all 4 slices and the harness step done. settings-input.py nl 161/162 and en 168/169, the one failure being the known kit-Dialog click bug (K1). settings-a11y.py nl 249/249 and en 260/260. Native tests 105/105. **Not done: the two review lenses and the fix.** |
| b | claude/settings-i-kit-ext | 594fd2c7 (WIP) | Step I package kit-ext (items K1-K7) on dcba2107. The WIP commit is unbuilt and untested. |
| c | claude/settings-c | 737ca765 (WIP) | Batch C (network & sharing): 3 slices, harness and two reviews done. **The fix was running.** Its WIP commit (51 files) is unbuilt and untested. The 13 findings it was fixing are in `helpers/c-review-findings.json`. |
| d | claude/settings-i-shell | 43cad15b (WIP) | Step I package shell (items S1-S8) on dcba2107. The WIP commit is unbuilt and untested. |
| e | claude/settings-i | dcba2107 | **Integration branch:** e56ab407 + merges of claude/settings-{b,d,e,f}. Clean. |
| f | claude/settings-i-host-settings | 382871cd (WIP) | Step I package host-settings (items H1-H26) on dcba2107. 1250aded (H1: libm.so.6 in the readelf recording) is a finished commit. The WIP on top is unbuilt and untested. |

Batches B, D, E and F are finished: slices, harness scenarios in nl and en, two reviews, a fix. Their handoff notes
are `notes/claude/settings-<x>.md` (in each branch). Every note has "Notes for I" sections: changes needed in frozen
or shared files, often as exact patches.

`git merge-tree` dry run, done at about 01:20: e56ab407 plus B, D, E, F, C and A (their tips at that time) merge
without a textual conflict.

Known base failure in every batch until H1 lands: `services::update::elf::tests::our_binaries_equal_readelf`. B0's
settings_mirror.rs uses powf, so the host binaries NEED libm.so.6. 1250aded fixes the recording.

## 5. The step I work list

`helpers/step-i-worklist.json` is the deduplicated list a collector built from every "Notes for I", contract §8 and
the open list. The packages split it by file area so they do not overlap.

* **kit-ext** (native/kit, extension/):
  * K1: real clicks never reach buttons inside a kit Dialog. High impact: the patch is in A's note 25 and B's note 9.
  * K2: controls on a SettingsCard or SettingsExpander header never get a click.
  * K3: panelBridge.js routing for launch.*, display.* and input.* (§8.1), plus the CallAsync patch.
  * K4: extension.js reads language "system" as Dutch.
  * K5-K7: breadcrumb folding, Dialog::set_primary_label, the dev tree's visibility.
* **shell** (native/shell, native/calendar, other apps):
  * S1: tray weekday and battery percentage (§8.2).
  * S2: taskbar.allDesktops (§8.3).
  * S3: the taskbar title after a live language switch.
  * S4-S5: notification banners.
  * S6-S7: wallpaper light/dark pair and GNOME slideshows.
  * S8: Start and caps (§8.6).
* **host-settings** (host/, native/apps/settings frozen files and the finished batches' pages, glossary):
  * H1: libm. **Done.**
  * H2-H26: Gs helpers, gsettings reset, the SIGTERM shutdown, the launch socket path, files/mod.rs language,
    H8 the two "Agenda" apps, H9 the Ubuntu Dock line (§8.4), registry notes, mirror fixes, glossary and others.
* **deferred** (DF1-DF29, after the packages):
  * Merge A and C.
  * Regenerate `native/shell/src/start/settings_pages.json` (`python3 -B tools/settings-pages.py --write`).
  * Remove `common::placeholder` and add a test that no page uses it.
  * VALIDATION_SETTINGS.md, the CHANGELOG under the existing 0.9.1 section (keep version 0.9.1), README.md and
    README.nl.md ("Known limitations": the display bullet is no longer read-only).
  * `tests/gnome/scenarios/settings-tour.py` (nl/en), and the check that fails on UI strings missing from
    tests/i18n/windows-glossary/*.tsv.
  * Harness helper fixes (DF10-DF13), scenario follow-ups once K1/K2/K3 land, A's grid.rs text-scale and tray
    input-indicator patches (DF15, DF16), contract corrections, and the full regression harness in nl and en.
* **robinOnly:** listed in the JSON, see section 7.

`helpers/batch-results.json` holds every batch agent's structured result: commits, tests run, open issues,
review findings. `helpers/batches-workflow.js` is the script that ran the batches; its review prompts (HOST AND
SAFETY, PAGE PARITY) are a good template for A's review.

## 6. Suggested order from here

1. **Batch C:** take the WIP 737ca765 in worktree c. Check it against `helpers/c-review-findings.json`, finish the
   fixes, build (`helpers/build.sh c`), run host and native tests, the checks, and settings-network.py and
   settings-sharing.py in nl and en. Commit (amend or follow up the WIP).
2. **Batch A:** run the two reviews (host and safety; page parity against plan.json, gnome-a/b.json and windows.json),
   then fix, rebuild and rerun settings-input.py and settings-a11y.py.
3. **The three packages:** finish them in worktrees b, d and f from their WIP commits, with the same build, test,
   harness and review loop. Each package may only edit the files listed for it in the work list. Several packages
   may add `notes/claude/settings-i.md`: concatenate the sections at merge.
4. **Integrate in worktree e on claude/settings-i:**
   * Merge the three package branches, then claude/settings-c and claude/settings-a.
   * Do the deferred items.
   * Run the full gates: build, host tests, native tests, checks, and every scenario under tests/gnome/scenarios in
     nl and en, plus settings-tour.py.
   * Do a final review and fix round.
5. **Bring it to the main checkout:** `git merge --ff-only claude/settings-i` on claude/gnome-english-update, or a
   merge commit. Then update notes/claude/plan.md (D-G77 completion) and the CHANGELOG.
6. **Report to Robin in Dutch,** with the Robin-only list.

## 7. Robin-only items (collect them for the final report)

* The go ("breng uit") to push and release 0.9.1 with this work, and to turn on immutable releases (repo Settings ›
  General › Releases).
* The go to delete the tag last-webkit.
* A clean-VM test of the installer's password dialog and the login-screen preselect.
* A real Windows 11 check of the wording rows marked verified=no in tests/i18n/windows-glossary/*.tsv.
  The libvirt VM "win11" on /srv/vm can serve for that.
* Decisions from the work list:
  * D-7: Alt+Tab default after the H12 mirror pair.
  * settings-e.md note 8: should settings-defaults.json's language become "system" for new installs?
  * H8: name GNOME Calendar "Agenda (GNOME)" / "Calendar (GNOME)"?
* Real-hardware and real-daemon checks no sandbox covers: night light on a GPU, backlights, three monitors, sound
  cards, NetworkManager 802.1X, a modem with a SIM, the hotspot, remote desktop, Flatpak/Snap permissions. Then a
  smoke test of the installed build in Robin's own session.

## 8. Other open items

* **Ubuntu 26.10 (GNOME 51) is not supported.** The extension lists shell-version ["50"] only, so GNOME 51 marks it
  OUT_OF_DATE. The WinPoint session then starts as bare GNOME, and install.py refuses a reinstall. Robin's
  /etc/update-manager/release-upgrades says Prompt=normal, so Ubuntu will offer 26.10 once Canonical opens upgrades.
  Claude advised Robin to stay on 26.04 and to use Settings › Windows Update › Meer opties › Nieuwe Ubuntu-versies ›
  "Alleen LTS-versies". The port is item G-X2 in notes/claude/map/updates.md. It needs a 26.10 VM, because the harness
  runs the host's GNOME 50.
* **Known gaps from before:** All apps shows two "Agenda" entries (H8), and a stale repo-root bin/ with WebKit-linked
  binaries that Robin removes by hand.
* `win11-gnome-showcase-en.mp4` in the main checkout is untracked and not Claude's: leave it.
