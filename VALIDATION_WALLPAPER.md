# Wallpaper and theme presets — 0.7.0

> **History (D-G72, 14 November 2026):** measured with the HTML interface in WebKitGTK of the time. It is gone: File Explorer, Settings, Task Manager and the other apps are native GTK 4 programs now, and WinPoint has no WebKit and no GTK 3 host any more. The Rust services this report checks still exist. The last commit with WebKit is tagged `last-webkit`; see [native/APPS.md](native/APPS.md).

The original Settings controls saved wallpaper choices, but the earlier GNOME port
had no original desktop surface and never applied those choices to GNOME's
background. The native desktop now draws the original wallpaper UI. A Rust
adapter also updates GNOME's underlying background, keeping the selected picture
or colour visible when the native desktop is unavailable.

The adapter uses the original built-in SVG assets and preserves custom local
photo paths. It maps the six original fit choices to GNOME, follows the original
Lotus/Flow light and dark twins (Lotus was called Bloom in 0.7.0; the old
builtin:bloom-* names still resolve to it), and applies solid colours. Explicit light/dark
choices work with system-theme sync disabled; automatic mode sets both GNOME
picture URI variants. GTK themes and the removed Fluent theme are unaffected.

Changes apply from every separate native app. GNOME updates are grouped and
unchanged values are skipped. Each effect reads the latest shared settings while
holding the same file lock as settings writes, preventing an older notification
from overwriting a newer wallpaper choice. The panel and desktop also apply the
saved background on startup. Background choices remain normal persistent GNOME
preferences.

Nineteen integration groups passed with the original Settings and desktop UI in
separate Rust/WebKit windows inside a private GNOME 50.1 session:

- All six theme cards changed the already-running desktop and both GNOME picture
  URI settings. The selected theme card and desktop light/dark mode agreed.
- The original background image tile, solid-colour dropdown and colour swatch
  worked; switching back to Picture restored the selected image.
- All six fit dropdown choices matched the original desktop CSS and GNOME values.
- The original follow-mode switch preserved the chosen light image when disabled
  and selected its dark twin when enabled after a mode change.
- The original Browse button applied an existing local PNG whose filename
  contained spaces, `#` and `%`; the desktop loaded it and GNOME stored the escaped
  file URI. Only the file chooser's returned path was supplied by the fixture.
- The original crossfade settled to one layer without retaining an obsolete image.

An independent `gsettings` process checked GNOME values under the fixture's private
HOME/XDG and settings backend. The active user session, wallpaper and index were
untouched. These checks ran within the combined two-monitor desktop suite.

Five Rust unit groups also passed: built-in mode/twin resolution, six fit mappings
and solid colours, escaped local file URIs and rejected invalid paths, filtering
relevant settings changes, and grouped writes observed by a second GSettings
client using an explicit memory backend.

Evidence: `validation/wallpaper-0.7.0.json`; integration helper:
`work/wallpaper-test/checks.py` in the working directory. The original Settings and
desktop JavaScript/CSS were not edited for this fix.
