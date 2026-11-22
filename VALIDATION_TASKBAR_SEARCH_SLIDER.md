# Taskbar input and Quick Settings slider, 22 November 2026

## Behavior

The full taskbar search box is now a GtkText editable. Its ordinary caret, selection, deletion and input stay in
the taskbar while Start mirrors the query and shows results. Clicking the Start field allows editing there and
mirrors the query back; returning to the taskbar preserves the query. A fixed 212 px allocation keeps long text
and the clear button from moving the taskbar app icons. Icon/compact/hidden search modes retain their controls.

The compositor has an explicit taskbar-search keyboard role, separate from other panel flyouts. Start's search
results map without taking the keyboard from the taskbar. Escape, outside clicks, launching a result and removing
the editable release that role. An explicit close restores the prior managed application when appropriate;
launching a chosen result or clicking a different application retains that application's focus. Delayed old
focus-loss callbacks cannot close a new valid search or steal input. Closing also clears only the panel's own
remembered text focus, preventing a later unrelated Start-button click from reopening Search before it toggles.

Normal Start keeps its own field, type-to-search behavior and Home view after clearing. The prior Start glass
restack correction is preserved; no global redraw workaround or session restart is introduced.

## Validation

The final real-input scenario passed **120/120 checks**: 46 taskbar/Start search checks followed by all 74
existing lifecycle checks in the same private compositor. The lifecycle section performs eighteen opens/closes
with the Start button, Escape and outside clicks, at three opening delays, with a deterministic pending-placement
change. The native process and compositor survived every cycle, and each Start backdrop was released.

The search checks cover empty results, all thirteen typed characters, Backspace, arrows, native caret placement,
clear, long text, Ctrl+A replacement, bidirectional field editing, Escape/previous-app focus, outside dismissal,
Enter launching a real Settings result, hidden/box changes, before-first-paint double activation, and normal
Start type-to-search/Home behavior. Explicitly restoring dock focus after closing Search does not reopen it,
while deliberate keyboard Tab into the editable still opens Search.

The complete harness gate passed: private install and all nineteen binary hashes/current extension verified,
normal inner exit, memory guard, safe traced connections/executables, no update operations, no leftover child
processes/sockets, unchanged source/binary/scenario hashes and removal of the owned temporary copy.

Separate release build of win11-shell: successful, with the two query-mode unit tests passing. Test executable
SHA256 fa1a9fa0ea79e62753d71429e51d4025973a42feeb4e4b7ed49761947f920770.

The current extension and test shell are installed only in a private temporary harness. The other eighteen
harness executables remain the 0.9.6 baseline. This verifies this change, not a complete rebuilt release.

The standalone slider fixture passed 23/23 checks in a separate private session. Both light and dark variants
measure an 18 px thumb and 4 px track, while keeping a 32 px scale and 40 px row. Real hover, press, an outside-
visual-thumb pointer target, Tab/Right/Home/End, disabled state and shared-slider geometry passed. Original
screenshots were visually inspected. GTK's assistive-technology backend is disabled in this fixture; no physical
touchscreen or assistive-technology compatibility claim is made.

No new binary is installed in the user's running session. No session setting, sign-out, restart or Git push was
performed. The mini-PC was not accessed.

## Evidence

- Final search/lifecycle: `/home/robin/Documents/Codex/2026-11-17/referenced-chatgpt-conversation-this-is-an/work/winpoint-final-search-close-20261122-213801-76d8e050/scenario-1/results.json`
- Final isolation and source stability: `/home/robin/Documents/Codex/2026-11-17/referenced-chatgpt-conversation-this-is-an/work/winpoint-final-search-close-20261122-213801-76d8e050/runner-results.json`
- Standalone slider: `/home/robin/Documents/Codex/2026-11-17/referenced-chatgpt-conversation-this-is-an/work/winpoint-final-input-slider-20261122-213013-c57f0877/scenario-3/results.json`
- Slider's own run passes all gates; its parent aggregate is false because an earlier search candidate failed.
  That search failure is repaired and superseded by the final 120/120 run above. Slider CSS and fixture are
  unchanged between that proof and the final source.
- Host grouping/icon work is documented separately in `VALIDATION_PROCESS_GROUPING.md`: 45 targeted Rust tests
  passed (one live-manager probe intentionally ignored), plus seven private installer checks.
