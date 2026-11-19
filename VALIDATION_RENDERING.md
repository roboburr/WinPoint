# Start rendering, 0.9.4

The user reported a vertically stretched web-search detail icon and best-result row, square coloured halos
around pinned icons, and flickering when opening dropdowns. The mini-PC runs Ubuntu 26.04 at 100% display scale.
The user clarified that the coloured haze appears only while the pointer hovers over an item.

The published 0.9.3 binaries reproduce the search allocation defect in an isolated GNOME 50.1 session:
the detail tile is 64 × 420 pixels and the best result is 489 pixels high. The glyph's vertical expansion was
propagating through its tile into the row and detail hero. Explicit non-expansion and centered alignment on
the tile keep the glyph centered inside a square without expanding its ancestors.

GNOME 50.1's `Shell.BlurEffect` in BACKGROUND mode copies the complete backdrop rectangle from the current
framebuffer. When only a damaged region has been repainted, the rest can still contain the previous foreground.
This can feed old icons and menus back into the blurred surface. The existing redraw-on-window-move workaround
does not cover icon hover, typing or dropdown changes. WinPoint now requests unclipped frames while any of its
background blur effects exist. It releases only a flag it set itself, preserving an already-enabled external flag.
It does not enable continuous redraw or add a repaint timer. Full-frame painting costs more per changed frame;
the framebuffer dimensions and number of blur effects do not increase.

Sources: [GNOME 50.1 blur implementation](https://github.com/GNOME/gnome-shell/blob/50.1/src/shell-blur-effect.c#L418)
and [upstream discussion of stale framebuffer pixels in partial blur redraws](https://github.com/aunetx/blur-my-shell/pull/1170).

`tests/gnome/scenarios/start-rendering.py` runs only in the private harness. The corrected binaries and extension
pass all 18 checks: five power-dropdown open/Escape-close cycles, real allocated geometry for web, settings and
app searches, redraw flag release/reacquisition, preservation of a pre-existing flag, and no continuous idle
painting. The detail tile is 64 × 64 pixels and the best result 72 pixels high; idle paint count is zero over two
seconds. Screenshots were inspected. Peak process-tree PSS is 512 MiB and minimum available host memory
4,925 MiB. Both connection and execution isolation checks pass; no processes or checkout sockets remain.

The physical mini-PC is not connected to this test environment. The coloured halos were reported in its photo
and did not appear in the isolated baseline screenshots, including actual pointer hovers. The redraw correction is verified against the source
mechanism and actual compositor state; confirmation on that mini-PC remains necessary after updating and signing
out/in. The existing Start teardown crash guard is retained.

The follow-up hover scenario uses a virtual pointer in the private compositor to point at Settings, Task Manager,
Calculator and LibreOffice. It verifies the pointer coordinates and a visible change inside each target, then
compares the settled hover (including its tooltip) to a forced full-frame reference. It also compares the pinned
area after the pointer leaves. Tooltips can extend outside their tile; the first pixel-difference fixture counted
those extensions as failures on both versions. The corrected comparison keeps the tooltip state unchanged.
All 31 follow-up checks pass. Each of the four hover/reference comparisons has zero changed pixels; leaving the
items removes the hover without a trail. Peak PSS is 509 MiB, minimum available memory 5,602 MiB, and both
isolation checks pass. The actual 0.9.4 release binaries also pass all 74 Start-close lifecycle checks and all 28
frosted-glass checks, including native window movement/resizing and the blur memory limit. Installer/release tests:
120 passed, two skipped.
