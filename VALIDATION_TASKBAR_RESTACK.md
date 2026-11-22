# Taskbar glass ordering during Start transitions

Opening or closing Start could briefly replace the taskbar, including its icons and tint, with untinted
wallpaper. The native panel stayed mapped and opaque, and its glass entry/effect were never recreated.
Ordinary screenshots missed the short transition because they paint the scene again.

Mutter restacks its window actors separately from plugin siblings. WinPoint previously repaired the sibling
order in a default-priority idle. A frame could be painted first, leaving the glass above the native client.
Mutter explicitly assigns the ordering of plugin actors to the plugin; see
[its stack synchronization](https://github.com/GNOME/mutter/blob/50.1/src/compositor/compositor.c#L516).
The idle path existed in both 0.9.5 and 0.9.6.

The correction queues one coalesced `BEFORE_REDRAW` callback on `restacked`. It restores only existing glass
siblings directly below their own client in the same parent. Desired entries, placement, source snapshots,
capability publishing and the GTK/Mutter Start frame gates retain their existing timing. Pending work is
cancelled through the same compositor-laters API during disable. See
[Mutter's redraw-aware later callbacks](https://github.com/GNOME/mutter/blob/50.1/src/compositor/meta-later.c#L236).

## Reproduction and comparison

A small test-only native observer reads the existing Cogl framebuffer on ordinary `after-paint` signals.
It does not call an actor/stage screenshot or paint API. Its preload is limited to the private harness's
GNOME process, guarded by its private HOME, runtime bus and run directory. The first resting readback is
calibrated against a screenshot. This is a single virtual monitor at 100% scaling; Mutter uses an offscreen
framebuffer for that virtual path, avoiding rotating onscreen backbuffer ambiguity:
[virtual framebuffer creation](https://github.com/GNOME/mutter/blob/50.1/src/backends/native/meta-renderer-native.c#L1462).

The original run captured 329 frames, including 9 frames with the taskbar foreground absent.
The corrected run observed 345 ordinary paints and captured 261 calibrated taskbar frames.
All eight Start open/close transitions passed: desktop, a maximized test window, a controlled overlapping
window, and WinPoint Settings. The foreground remained visible and every sampled background region stayed
pixel-identical during each transition. The final test disarms each capture before changing the next context
and updates its masks for the current taskbar buttons. A permanent `after-paint` regression now checks the
same glass-below-client invariant in `start-avatar-background.py`, without requiring the native helper.

## Existing private GUI regressions

- start-avatar-background: 15 passed
- start-close-lifecycle: 74 passed
- glass-window-content: 12 passed
- frosted-glass: 28 passed
- multi-monitor: 75 passed
- start-rendering: 32 passed

**236 checks passed**, in addition to the focused framebuffer experiment. All runs passed their connection
and execution isolation checks and left no private processes behind. Existing 0.9.6 WinPoint binaries were
used; only the small measurement helper was compiled. The user's installed files and running desktop were
not changed or restarted, and nothing was pushed or published.

The multi-monitor log contains the same four pre-existing GNOME workspace-clone `addWindow` errors during
private lock/session-mode teardown as the unmodified 0.9.6 baseline. Their signatures and counts are unchanged;
all 75 scenario checks passed. No new JavaScript error signatures or source-removal errors were found.

Physical confirmation on the user's two displays remains a post-installation check. This validation proves
the reproduced compositor-ordering defect and its correction in the private GNOME/Mutter 50.1 environment.
