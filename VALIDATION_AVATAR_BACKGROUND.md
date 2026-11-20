# Start avatar and stable desktop glass, 0.9.5

The user reported an oversized profile photo in Start, followed by apparent refreshing when opening/closing
Start and dropdowns in Settings or other apps. They confirmed 0.9.4 after signing out/in; the flicker may affect
the background or the whole image. The physical mini-PC is not connected to this environment.

The profile defect is reproduced using a synthetic 1600 × 900 account picture in the private HOME. On 0.9.4 the
Start avatar allocates 270 × 152 pixels. A GtkPicture's natural image size was influencing the footer despite its
32-pixel minimum. The photo now paints as an unmeasured, clipped overlay on a 32 × 32 base. The test verifies
actual allocated rectangles and a loaded picture, not just requested dimensions. The source photo is unchanged.

The private trace shows that opening/closing Start and four real Settings dropdowns retains the taskbar actor,
opacity, blur entry and effect. It does not reproduce the physical screen flash. Nevertheless, the old blur reads
the live foreground framebuffer, and the 0.9.4 workaround changes global clipped-redraw behavior. Both are removed.

Glass now blurs a shared desktop snapshot per monitor in ACTOR mode. Mutter's `paint_to_content` captures only
the desktop, with culling inhibited for that offscreen capture; it does not read the screen containing the UI.
Desktop damage refreshes that source after a 150 ms debounce. Source signals, timers and contents are released
when no backdrop uses them. There is no polling timer or global drawing flag. The taskbar sample is clamped at
the desktop edge because Mutter limits desktop windows to the work area above the taskbar. Oversized/spanning
windows cover their allocation. This intentionally reflects the desktop instead of other app windows.

An early clone prototype left transparent pixels below the shortened desktop and failed a bright-background
pixel test. The final cached, bounded source passes: inserting a bright red actor beneath the taskbar changes
zero pixels in its glass. The candidate passes all 11 profile/dropdown/background checks. Peak PSS is 510 MiB;
minimum available host memory is 5,299 MiB; connection and execution isolation checks pass.

The permanent scenario `tests/gnome/scenarios/start-avatar-background.py` uses
`--profile-picture tests/gnome/fixtures/avatar-wide.png`. The harness accepts only a fixture inside the checkout
and copies it to its private HOME. Follow-up checks cover cache cleanup, shared reuse and idle capture counts.
`start-rendering.py` retains its actual pointer hovers and pixel references while checking that global redraw flags
stay unchanged. The existing Start-close and frosted-glass regressions remain required, together with two-monitor
wallpaper and taskbar checks. Physical mini-PC confirmation after installing remains necessary.

Implementation references: [Mutter's offscreen window content API](https://github.com/GNOME/mutter/blob/50.1/src/compositor/meta-window-actor.c#L1673)
and [GTK Picture sizing](https://docs.gtk.org/gtk4/class.Picture.html).

Final GUI validation of the 0.9.5 rendering build: **223 checks passed**: avatar/background 14, hover/search 32,
Start-close lifecycle 74, frosted glass 28, and two-monitor behavior 75. The avatar test also verifies idle capture
counts, releasing all cached sources on disable, and sharing one source after re-enabling. All connection and
execution isolation checks pass, with no leftover processes or checkout sockets. Installer/release checks:
120 passed, two skipped. The later Task Manager grouping change has its own API/process regressions and does not
change the renderer or native binaries.
