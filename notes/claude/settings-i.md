# Settings parity, step I

Current status: integrated acceptance completed on 17 November 2026; see the final section below. Earlier package WIP notes are preserved as history.

## Package shell

Branch `claude/settings-i-shell` from `dcba2107` (B, D, E and F merged), worktree `win11-gnome-wt/d`. Only the
shell files of the package changed (`native/shell/src/panel/taskbar/{system,util,apps}.rs`,
`native/shell/src/panel/{taskbar,mod}.rs`, `native/shell/css/taskbar.css`,
`native/shell/src/flyouts/{notifications.rs,menus/taskview.rs}`, `native/shell/src/desktop/wallpaper.rs`,
`native/shell/src/start/search.rs`).

* **S1** (contract 8.2; settings-d harness note 7, settings-e I-note-2). The tray clock: with `time.showWeekday` the
  date line carries the abbreviated weekday in WinPoint's language, not the process locale (`util.rs`
  `fmt_weekday_short` / `fmt_date_short_weekday`: "di 17-11-2026", "Tue 17/11/2026"); with the date off the weekday
  goes before the time ("di 14:05"), as GNOME's clock does with clock-show-date off. The battery: a label
  `tb-battery-percent` (class `w11-battery-percent`, 12 px, 2 px from the glyph, tabular figures) after the battery
  glyph, in one box with it, "76%" (`percent_text`, the rounded charge) while `taskbar.batteryPercent` and a battery
  is present; it follows `settings.changed` (`taskbar.batteryPercent`, and `time` for the clock) and
  `system.battery.changed`.
* **S2** (contract 8.3, amendment 12; settings-d slice 3). `taskbar.allDesktops` false (Windows' default, also when
  the key is missing) shows on every taskbar only the windows of the desktop in use and the sticky ones (`pinned`);
  a pinned app whose windows are elsewhere stays as a pin. The desktop in use is the extension's `activeWorkspace`
  with windows.list and windows.changed (stored with the windows, `Catalog::set_windows`), else the focused
  window's, else the most recently focused window's (a focus history from windows.changed and windows.focused),
  else unknown (every window). `desktop_in_use` and the window test `shows_on_desktop` now live in `apps.rs` and are
  re-exported by `taskbar.rs`; Taakweergave (`taskview.rs`) uses the same two (its `desktop_in_use` is a re-export,
  its `on_desktop` a filter over `shows_on_desktop`), because `menus.rs` keeps `mod taskview` private, so the
  taskbar could not reach the old place. A settings.changed of `taskbar.allDesktops` renders every taskbar.
* **S3**. The main taskbar window takes "Taakbalk" / "Taskbar" again on every settings.changed when its title
  differs (`panel/mod.rs`), as `strips::retitle` does for the other monitors.
* **S4** (settings-f slice 2 note 3). A notification that arrives `expanded` (GNOME's "Inhoud weergeven in banners",
  force-expanded, or a critical one) shows up to 12 body lines in the toast and in the centre's card
  (`body_lines`); others 3 and 4 as before.
* **S5** (settings-f slice 2 note 9). `notifications.consumerReady` (every 2 s) and `notifications.delivered` carry
  `sounds` and `toasts` (`notifications.sounds` / `notifications.showToasts`, on unless false), so the bridge needs
  no settings file.
* **S6** (settings-f slice 2 note 1, the shell half). `resolve_path`: in dark mode with `followTheme` on, a
  non-empty `wallpaper.pathDark` is the picture (an old built-in id in it is renamed as in `path`); without the key
  (old settings files), with an empty one or with followTheme off, as before. The key itself and the writer are
  H17's.
* **S7** (settings-f slice 2 note 2). A `.xml` wallpaper (any case, a path or a file:// URI) is read as GNOME's
  slideshow: a small parser for the format of gnome-desktop's gnome-bg-slide-show.c (`<starttime>`, `<static>` with
  `<duration>` and `<file>` or its `<size width height>` children, `<transition>` with `<from>`/`<to>`, comments,
  CDATA, entities; no crate added), then the slide that is current now (the time since the start in local time
  modulo the total of the durations; a transition shows the picture it is closer to), the size of it that suits the
  monitor as GNOME's find_best_size, and only absolute paths; a slideshow naming another `.xml` shows nothing. The
  desktop takes the picture current when it loads (a theme, size or wallpaper change loads again); it does not
  switch slides by itself on a timer.
* **S8** (contract 8.6). Start's Instellingen results leave out a page whose `needs` (a `needs` field of
  settings_pages.json) caps.get reports false; caps.get is asked once when some page has `needs` and followed through
  caps.changed (results render again when the caps change); without an answer, or for a key caps does not know,
  every page stays. Inert until the JSON gains the field (DF2: tools/settings-pages.py and the regeneration).

### Tests

WIP 43cad15b and the Codex follow-up have not been built or runtime-tested. Source-only validation on 17-11:
Rust syntax for the changed shell files, check_pairs (0 errors/0 warnings), check_native (0 errors, the existing
48 native-dutch-only warnings), and git diff --check passed. The new GTK measurement regression below is added,
not executed. No live session API, installation, build or push was used.

### Not done here (other files)

Host-settings supplies the card-content helper correction, theme writes clearing pathDark, shared reset safety,
and the regenerated Start settings page list with needs fields (DF2). A/C and the kit package still need integration.
All release builds, Rust/native tests and nl/en isolated GNOME harness runs remain required when permitted.


## Codex source repairs, 17-11-2026

* S-R1: resolve_path now ignores a leftover pathDark when the selected path is built-in, matching the host and
  Settings resolver. The regression cases cover a stale custom dark photo with a newly selected built-in theme.
  host-settings also clears pathDark when applying a theme preset.
* Reported oversized Quick settings panel: large MPRIS media artwork had a GtkPicture whose natural source size
  could widen the entire flyout despite its 360 px minimum. Artwork is now painted in a DrawingArea with card-sized
  content dimensions (300 px wide, at most 180 px high); cover scaling affects painting only. Added a GTK measurement
  regression with a 1920x1080 source texture, not run. This corrects source only: the running panel has not been
  rebuilt, restarted or replaced, so the screenshot's live session is untouched.
* No runtime pass is claimed. Commit remains WIP until the permitted build and isolated test loop completes.


## Final integrated acceptance, 17 November 2026

This supersedes the historical WIP/source-only status above and the package repair checkpoint notes. All A-F branches
and the three integration packages are merged. K1-K7, S1-S8 and H1-H26 have their integrated implementation and deferred
follow-ups; the four Codex review repairs are included. DF1-DF29 are accounted for by the code, API contract, registry,
wording gates, isolated tests and explicit feature boundaries in VALIDATION_SETTINGS.md. Optional absent services and
physical-hardware validation are not claimed as implemented/tested merely because their pages can be visited.

Legacy scenario callers now await the public CallAsync bridge and verify actual activation tokens. The language
scenario waits for GNOME’s separately updated application-name cache after switching its initial seed language.

Additional repairs found in the final loop include GNOME 50 modal-grab compatibility for shortcut capture, typed power
profile fixtures, real scrolling inside modal lists, effective GSettings default readback, public Cracklib data mounted
read-only for the actual password-quality test, and revealing collapsed sections before inspecting their children.
The file-picker regression runs in the product’s WinPoint session mode: its portal choice intentionally leaves GNOME’s own session untouched.
The app chooser and privacy tests reject fallback clicks rather than masking a failed real pointer action.
The extended native-shell test additionally moves a real unpinned window between workspaces, checks both taskbar
filters and live taskbar-title language changes, measures an eight-line expanded toast and samples the real desktop pixels for a light/dark image pair.

VPN import now uses offline libnm parsing with interactive AddConnection2; APN attach credentials and authentication
flags are typed and secret-aware. See settings-c.md's integrated follow-up for the tested route and remaining boundaries.

Builds passed (19 staged release programs), host 824/0, native 250/0, private bus C16/E3, three actual GTK regressions,
installer 119 passed/3 explicit skips and five harness unit tests. 68 isolated runs, 4695 individual checks. All normal trace/isolation/cleanup
gates pass. The only accepted physical limit is colord refusing calibration for the headless Meta virtual monitor.
The late-socket guard test deliberately aborts its private session before connecting to the newly visible outside socket.

No live installation, user-session setting change, shell restart, administrator password, push, tag deletion or release
was performed. Preserve the user's untracked showcase video. Release/installation, clean-VM password/login checks,
verified=no Windows wording checks and real hardware/daemon tests remain separate user actions.
