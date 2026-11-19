# Host-settings: Codex source repair checkpoint, 17-11-2026

Branch claude/settings-i-host-settings, starting at WIP 382871cd (H1 already in 1250aded).
Robin requested the four review repairs and failing light checks, without builds, live-session interference or
pushes. The oversized Quick settings report was handled in the shell package, also as source-only work.

## Repairs

* K-R1 integration dependency: common::card_text_box follows SettingsCard::button().child() for clickable cards,
  so the kit's new Overlay root does not swallow card_extra content such as storage bars. Plain cards keep their
  existing traversal. Added a GTK test for both card kinds, not run.
* S-R1 integration dependency: theme presets explicitly clear wallpaper.pathDark. The shell also ignores a stale
  pathDark for built-in selections, matching the host and Settings preview.
* H-R1: reset_as checks the effective default against env.held() before any key resets, including dry-run. It
  returns gsettings.heldByWinPoint with the same key/accel/setting data as normal writes. Added memory-backend
  regressions for Alt+Tab, Ctrl+Alt+Delete, dry-run, whole-call refusal and successful reset after releasing the
  extension binding; these Rust tests are source only, not compiled or run. The follow-up on A adds the same guard
  to its current local reset helper and makes resetAll skip held defaults; preserve that error handling at DF19
  when switching to the shared API. The guard deliberately does not disable or steal WinPoint's binding.
* H-R2 / H25: AudioMonitor keeps supervising after the first Pulse connection. A later disconnect starts the
  private pw-dump fallback, recovery stops and reaps it, and the same supervisor supports another loss/recovery.
  Fallback notifications are suppressed while Pulse is ready; retries back off, with shutdown/recovery interrupting
  the delay. Added a two-cycle FakePulse test, not run. No pw-dump or audio client was started during this work.

## Light-check repairs

* Formatted the ten host files previously failing rustfmt; the changed audio monitor and reset source are formatted
  as well. The existing 30 formatting exceptions elsewhere remain unchanged.
* check_native accepts empty keyword lists when both languages are empty, while rejecting a list in only one
  language. Added a regression for both empty, both non-empty, Dutch only and English only.
* The broader source checks exposed the deferred Start-list drift. registry_pages now exports NEEDS and the
  generated settings_pages.json is synchronized with this branch's registry (85 pages). This completes the source
  part of DF2 for the current tree and activates S8 once merged with shell. Regenerate again after A/C integration.
  Added a test of the exported requirements and detection of their drift.
* The glossary-file test now also checks the existing i.tsv from this package, rather than failing because its
  hardcoded file list predates package I.
* The additional drift check is clean too: H7's removed technical en/nl literals are explicitly recorded as
  centralised in Lang::of, and H22's Dutch/English Chinese script labels now use machine.tsv through the normal
  catalogue API (created by extract_rs.py, with new-key baseline records). Their native Chinese endonyms remain
  separate. No unrestricted allow-list exception was added.

## Validation and remaining gates

Host rustfmt: 229 files, 0 failing, 30 existing exceptions. check_pairs: 0 errors/0 warnings. Native i18n: 0 errors,
48 existing native-dutch-only warnings. Full i18n dispatcher: 0 errors, 105 warnings (48 native + 57 Rust).
settings-pages check and git diff --check pass. drift_rust reports 0 errors/15 existing warnings. The source-only
no-WebKit check passes for both changed packages, with binary checks deliberately skipped. Rust source parses with
rustfmt; this does not type-check it.
All 13 Python native-checker/registry/glossary tests pass on source and private fixtures under the Codex workspace;
no application or service is launched.

No builds, cargo/native tests, GTK tests, GNOME harness, D-Bus fakes, installations, live-session calls or pushes.
The new GTK, reset and reconnect tests must run later inside the prescribed sandboxes. Runtime behaviour, including
the current live panel size, is not verified by this checkpoint. This branch remains WIP, not release-ready.
