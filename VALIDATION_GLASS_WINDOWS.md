# Underlying-window glass and Start frame lifetime, 0.9.6

0.9.5 deliberately replaced live background-framebuffer blur with a desktop snapshot to avoid feedback and
partial-redraw artifacts. The user showed that this displays wallpaper even when Start overlays a browser.
A second screenshot showed the independent blur rectangle without Start's native tint/content during opening
or closing. This release restores underlying-window content while keeping framebuffer feedback excluded.

The actor-mode blur now composes the desktop and visible, intersecting MetaWindowActors below its client,
using their actual sibling stacking order. Each window has one shared GPU content snapshot, updated on damage
with a 33 ms coalescing timer. Geometry, visibility and stacking changes update the layers; unused snapshots,
signal handlers and pending timers are released. There is no idle polling or global redraw flag. The client
itself and sibling backdrop widgets cannot enter their own source. Desktop-edge clamping remains for taskbars.

Start sends its open phase after GTK's after-paint, guarded by the current open/close epoch. The extension also
waits for Mutter's first-frame signal on a newly managed Start actor. Existing windows when enabling the
extension are already drawn. Its independent glass is synchronously suspended on compositor visibility,
opacity and transform changes and is eligible only for a stable, mapped, visible actor. This prevents a
late or disappearing client from exposing a standalone blur rectangle.

The permanent glass-window-content scenario uses real private GTK windows, not a mock window list. It changes
a red window to green while Start stays open and checks the resulting glass pixels; opens a second blue app,
checks its top position, raises the green app and checks the new stack; moves a source to another workspace;
minimizes and closes it; and verifies source cleanup. It also makes a Start compositor fade deterministic and
samples real repeated opening frames, rejecting the bright untinted source rectangle.

Final isolated GUI checks passed:
- glass-window-content: 12
- start-avatar-background: 14
- start-rendering: 32
- start-close-lifecycle: 74
- frosted-glass: 28
- multi-monitor: 75

The window-glass PSS increment in the isolated GNOME measurement was 20.2 MiB, below the 64 MiB test limit.

**Total: 235 checks passed.** Connection and execution isolation passed in every run;
there were no leftover private processes. The native shell and all ten versioned host programs were built in
release mode. The remaining eight native binaries are unchanged from 0.9.5 and are byte-compared when staging.
No installer code changed; the preceding 120-pass/two-skip installer validation remains applicable. Actual
installer preflight, package integrity and public update availability are checked separately before completion.

These tests do not drive or restart the user's desktop. Physical display flicker on the mini-PC remains a
hardware confirmation after installing and signing out/in; it is not claimed reproduced by the private renderer.

API references: [Mutter window content snapshots](https://github.com/GNOME/mutter/blob/50.1/src/compositor/meta-window-actor.c#L1673),
[Mutter first-frame semantics](https://github.com/GNOME/mutter/blob/50.1/src/compositor/meta-window-actor.c#L195),
and [GNOME Shell actor blur](https://github.com/GNOME/gnome-shell/blob/50.1/src/shell-blur-effect.c#L32).
