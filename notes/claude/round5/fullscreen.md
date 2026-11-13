# R5 fullscreen: taskbar, Start and Alt+Tab over a fullscreen app

Worktree `.wt/fs`, branch `r5/fullscreen` from 7efc1e61 (0.8.0). Changed: `extension/panelBridge.js` only (plus this
note and `tests/gnome/scenarios/r5-fullscreen.py` with its outputs). No Rust, UI, text, setting or schema change;
`extension/shortcutBridge.js` did not need a change.

Robin: "ik wil in fullscreen (fullscreen windowed) apps kunnen alt tabben, en als ik op windows icon op toetsenbord
druk, wil ik de taakbalk kunnen zien ook, nu zie ik alleen start menu en kan ik niet wisselen van apps".

## Cause (verified in the harness)

Probe `tests/gnome/scenarios/r5-fullscreen.probe1/probe1.py` (run from `.rt/fs/`; its output in the same directory),
base-debug build, the tree before the change, real keys through a Clutter virtual keyboard:

* `_hidden()` inhibited the panel window (`inhibit_mapped`) whenever `get_monitor_in_fullscreen(primary)` was true.
  With File Explorer fullscreen and Start opened by a real Super press, mutter still reports the monitor in
  fullscreen (Start, 642×726, is not maximised, so it does not "obscure" the monitor) and the panel stays inhibited:
  Start without a taskbar (step 03).
* A real Alt+Tab opens the task view in the panel page (`is-open` true, the panel even gets the keyboard focus), but
  that page lives in the inhibited panel window, so nothing is visible (step 11 before the manual uninhibit).
* Mutter 50 keeps the DOCK panel in its own layer (4) above the fullscreen window (layer 2) also while the monitor
  is in fullscreen; there is no demotion of docks. Uninhibiting alone shows the taskbar above the fullscreen app
  and above Start (step 04, screenshot `04-uninhibited-dock.png`), and the task view over it (`11-alt-tab.png`).
  So no type change, layer trick or actor restacking is needed.
* Second finding: after Esc in the task view the keyboard focus stays on the panel (step 12; also out of
  fullscreen, step 13: an old quirk). With Windows' rule that would keep the taskbar up and leave the fullscreen app
  without keys, so Esc needs a focus return.
* A monitor-sized request of an undecorated Wayland window (`move_resize_frame` to 1440×900) ends as 1440×852:
  mutter keeps it to the work area and never calls it fullscreen (step 14).

## Change (`extension/panelBridge.js`)

Windows 11's rule: over a fullscreen app the taskbar is hidden only while that app (or one of its transients) has
the focus and no Start, task view or other taskbar flyout of ours is open or about to open.

* `_hidden()` = no unlocked user session, or the overview, or (primary monitor in fullscreen and not
  `_showOverFullscreen()`). `_showOverFullscreen(monitor)`:
  * false when no managed user window on that monitor and the active workspace `is_fullscreen()`: a monitor-sized
    override-redirect window also makes mutter's monitor fullscreen, and nothing can be stacked above it, so the
    old rule (hidden) stays there;
  * true while a reveal request is pending (`_revealUntil`), a taskbar flyout or menu holds the keyboard
    (`_panelExclusive`, from `panel.keyboard`), or a Start window exists (`_startOpen()`);
  * otherwise true unless the focus window, followed up its `get_transient_for()` chain, is a fullscreen window
    on that monitor (no focus: hidden, as before).
  "Fullscreen windowed" apps follow the same rule whenever mutter treats them as fullscreen (`is_fullscreen()` and
  `get_monitor_in_fullscreen`).
* Re-evaluated on `notify::focus-window` (which also ends a pending reveal), `in-fullscreen-changed` (as before),
  Start's `shown` and `unmanaged`, `panel.keyboard` going back to on-demand, and the reveal timeout.
* `_requestReveal()`: called by `_toggleStart` when no Start window is open (Super through overlay-key, and the
  taskbar's Start button) and by `_openTaskView` (Alt+Tab through the task-view keybinding) **before** the D-Bus
  call to the panel or Start, so the panel window is mapped above the fullscreen app before the task view or Start
  draws. It only acts while the primary monitor is in fullscreen. The next focus change or `REVEAL_MS` (2000 ms)
  ends it, so a request whose surface never appears cannot keep the taskbar up.
* Esc in the task view over a fullscreen app: `_openTaskView` arms a focus return; when the panel then takes the
  keyboard (`panel.keyboard {exclusive:true}`) the window it came from is remembered; when the panel gives the
  keyboard back and still has the focus `RETURN_MS` (400 ms) later, that window (else the topmost user window on
  the workspace) is activated again, so the app has the keys and the taskbar hides. A window picked in the task
  view arrives well within the 400 ms and wins. Only for the task view the extension opened, and only while the
  monitor is in fullscreen: elsewhere the focus stays as it always was.
* Struts unchanged: `_syncStrut()` uses `_strutHidden()`, the old rule (session, overview, monitor in fullscreen),
  so a taskbar shown over a fullscreen app does not shrink the work area and reflow the windows below it.
* Start's place: without the strut the work area is the whole monitor, so `_positionStart` put Start 48 px lower
  (bottom 888) and the revealed taskbar covered its user and power row (seen in the first run's screenshot). While
  the taskbar shows without a strut, `_positionStart` now ends the work area at the taskbar's top edge, so Start
  sits where it sits out of fullscreen (bottom 840). Nothing changes when a strut exists.
* Frosted and notification bridges: still synced first in `_syncVisibility()` and both read `_hidden()` (the frosted
  taskbar backdrop and the Win11 toasts follow the visible taskbar). After an inhibit change a frosted sync is
  queued, because the sync before it ran while the panel actor was still hidden. The inhibit flag is set before
  calling mutter, so a focus change emitted from inside that call cannot inhibit twice.
* `disable()` removes the two new timeouts; `_forgetPanel()` clears the keyboard and return state.
* Task Manager always-on-top (`TaskManagerOnTop` in extension.js), shortcutBridge, the fallback St bar and GNOME's
  top bar handling are untouched. GNOME Shell 50 APIs only (`get_monitor_in_fullscreen`, `is_fullscreen`,
  `get_transient_for`, `located_on_workspace`, `sort_windows_by_stacking`, `inhibit_mapped`).

## Scenario `tests/gnome/scenarios/r5-fullscreen.py`

Real keys through a Clutter virtual keyboard in the private shell (Super_L, Alt_L+Tab, Escape; the direct handler
call is a recorded fallback that was never needed), real pointer clicks on taskbar buttons and task view cards
without focusing the panel first. Task Manager and File Explorer are open; Explorer is made fullscreen with
`Meta.Window.make_fullscreen()`. Stacking is compared in mutter's order (`sort_windows_by_stacking`) and in the
window group's actor order.

Result: **31/31 in nl and 31/31 in en** (`r5-fullscreen.out/`, `r5-fullscreen.en.out/`; 17.8 s of scenario, runs of
127 s and 124 s, peak PSS 1165/1137 MiB, 0 update-tool calls, 0 sockets left). Every key arrived as a real key (no
handler fallback). Numbers from the nl run (en within 0.05 s):

| # | Check | Result | Numbers |
|---|---|---|---|
| 1 | File Explorer and Task Manager are open, File Explorer is focused, the taskbar shows | pass |  |
| 2 | A: File Explorer is fullscreen and focused, mutter reports the primary monitor in fullscreen | pass |  |
| 3 | A: over the focused fullscreen app the taskbar is hidden (panel inhibited, actor not visible) | pass |  |
| 4 | A: no strut while the monitor is in fullscreen (as before) | pass |  |
| 5 | A: Task Manager is below the fullscreen File Explorer | pass |  |
| 6 | B: Super opens Start over the fullscreen app (Start mapped and focused) | pass | seconds 0.09, via Super key |
| 7 | B: the taskbar shows with Start (panel window mapped, not inhibited) | pass |  |
| 8 | B: the panel window is stacked above the fullscreen File Explorer and above Start | pass |  |
| 9 | B: Start sits above the taskbar, where the strut would put it (its bottom row is not behind the bar) | pass |  |
| 10 | B: mutter still reports the monitor in fullscreen; no strut is added (struts as before) | pass |  |
| 11 | B: the taskbar has its frosted backdrop directly below the panel actor (frosted bridge synced) | pass | seconds 0.0 |
| 12 | B: the screenshot shows the taskbar: the Start button area changed against the hidden state | pass | changed 0.228, samples 400 |
| 13 | C: Esc closes Start and File Explorer (fullscreen) has the focus again | pass |  |
| 14 | C: the taskbar is hidden again | pass | seconds 0.16 |
| 15 | C: the screenshot is the hidden state again (Start button area as in A) | pass | changed 0.0, samples 400 |
| 16 | D: a click on Task Manager's taskbar button focuses it and closes Start | pass |  |
| 17 | D: the taskbar stays visible, above Task Manager and the fullscreen File Explorer | pass |  |
| 18 | D: the monitor is still in fullscreen (Task Manager is smaller), so the new rule decided | pass |  |
| 19 | D: a click on File Explorer's taskbar button focuses the fullscreen app and the taskbar hides | pass | seconds 0.0 |
| 20 | E: Alt+Tab over the fullscreen app opens the task view (panel page) | pass | via Alt+Tab keys |
| 21 | E: the panel window is mapped above the fullscreen File Explorer and has the keyboard | pass | seconds 0.27 |
| 22 | E: the task view lists File Explorer and Task Manager | pass |  |
| 23 | E: Esc closes the task view, File Explorer has the keyboard again and the taskbar hides | pass | seconds 0.42 |
| 24 | F: picking Task Manager in the task view focuses it; the taskbar stays visible above both windows | pass |  |
| 25 | F: picking the fullscreen File Explorer in the task view focuses it and the taskbar hides | pass | seconds 0.0 |
| 26 | G: a reveal request (the start of Super and Alt+Tab) shows the taskbar at once | pass | seconds 0.0 |
| 27 | G: with nothing opening it ends by itself after about 2 s (no focus change) | pass | seconds 2.04 |
| 28 | G: the next focus change ends the request: back on the fullscreen app the taskbar hides well before 2 s | pass | seconds 0.06 |
| 29 | H: out of fullscreen the taskbar shows, with its strut, above File Explorer | pass |  |
| 30 | I: a monitor-sized request of an undecorated Wayland window is no fullscreen to mutter (kept to the work area), so the taskbar stays, as before | pass |  |
| 31 | every check passed | pass |  |

Timings (key or click to the state, polled every 50 ms): Super to Start focused with the taskbar 0.09 s; Esc in
Start to File Explorer focused and taskbar hidden 0.24 s; taskbar click to Task Manager focused 0.16 s; Alt+Tab to
the task view with the taskbar 0.27 s (nl) / 0.27 s (en); Esc in the task view to File Explorer focused and hidden
0.55 s (the 400 ms RETURN_MS included); reveal request alone ends after 2.04 s; with a focus change 0.06 s.
Stacking at B (mutter's order, bottom 0): Desktop 0, Task Manager 1, File Explorer 2 (layer 2, fullscreen), Start 3
(layer 2), panel 4 (layer 4, DOCK); actor order the same, frosted taskbar backdrop right below the panel actor.
Start's frame at B: [399, 114, 642, 726] (bottom 840, above the taskbar top 852), the same place as out of
fullscreen. Part I: the 1440×900 request ends as [0, 0, 1440, 852], `is_fullscreen` false, `is_monitor_sized` false.

**The same scenario on the unchanged tree** (`--source 7efc1e61`, `r5-fullscreen.before.out/`): 15 checks, 5 failed,
then stopped: Super opens Start (check 6 passes) but the panel stays inhibited (taskbar not shown, not above,
no frosted backdrop, Start button area unchanged: 0.0), Start sits at [399, 162, 642, 726] (bottom 888, the row
that the taskbar would cover), and part D stops with "the taskbar is not visible (panel window not shown): nothing
to click", which is Robin's "kan ik niet wisselen van apps".

## Screenshots

In `tests/gnome/scenarios/r5-fullscreen.out/` (and `.en.out/`, same names; PNGs stay out of git, D-G22):
`a-fullscreen-focused` (taskbar hidden), `b-super-start-and-taskbar` (Start and taskbar over the fullscreen app),
`b-taskbar-strip` (the 48 px strip), `c-start-closed`, `d-task-manager-over-fullscreen` (another app picked on the
taskbar), `d-back-to-fullscreen`, `e-alt-tab-task-view` (task view over the fullscreen app), `e-escape`,
`f-task-view-picked-task-manager`, `f-task-view-picked-fullscreen`, `h-out-of-fullscreen`, `i-monitor-sized-window`.
Before the change: `r5-fullscreen.before.out/b-super-start-and-taskbar.png` (Start only, no taskbar). Probe:
`r5-fullscreen.probe1/*.png` (the script is kept next to it as `probe1.py`).

## Regression runs (tests/build/base-debug, `--source worktree`)

After the last change, each `tools/heavy.sh --min-mb 2560 -- python3 -B tests/gnome/harness.py launch run --lang nl
--build-dir tests/build/base-debug --source worktree … --out tests/gnome/scenarios/r5-fullscreen.regress/<name>`:

| Scenario | Result | Run |
|---|---|---|
| ge1c-extension.py | 64/64 | 134.5 s, peak PSS 979 MiB |
| r3-ontop.py (nl, then en after its live switch) | 31/31 | 134.5 s, peak PSS 1145 MiB |
| cross-r1.py with `--surfaces desktop,panel,start,explorer,settings,taskmgr,alles,disks` | 133/133, 15 surfaces 0 failed | 250.5 s, peak PSS 1140 MiB |

All with 0 update-tool calls, 0 sockets appeared, 0 left in the checkout. The same three also passed (64/64, 31/31,
133/133) on the version before the Start-position change. Static checks:
`PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check.py`: 0 errors, 53 warnings (rust 53, the
round-4 count); `python3 -B tools/i18n/check_pairs.py`: 0 errors, 0 warnings; `node --check` on the module: ok.

## What Robin should try live (after the merge and the deploy recipe's install and re-login)

1. A fullscreen app: Firefox with F11, a video player in fullscreen, a game in fullscreen and one in "fullscreen
   windowed"/borderless mode. Taskbar hidden while you play.
2. Windows key: Start and the taskbar appear above the app. Click another app on the taskbar: it comes to the
   front, Start closes, the taskbar stays. Click the game's taskbar button: the game is in front and the taskbar
   hides. Windows key, then Esc: back in the game with the keyboard, taskbar hidden.
3. Alt+Tab in the game: the task view appears above it. Pick another window: it comes to the front with the
   taskbar. Alt+Tab, pick the game (or press Esc): the game has the keyboard and the taskbar hides.
4. With two monitors: focus a window on the other monitor while the game is fullscreen on the primary: the taskbar
   shows over the game (Windows does the same); click the game: hidden again.
5. A notification while the taskbar shows over the game appears as a Win11 toast; while the game has the focus,
   GNOME handles it as before.
6. Report a game where Super or Alt+Tab does nothing (see the limits), or where the taskbar stays up.

## Limits

* Apps that take the system shortcuts get Super and Alt+Tab themselves, so nothing of ours opens: Wayland
  keyboard-shortcuts inhibitors that GNOME allowed (virtual machines, remote desktop, some SDL games), and Xwayland
  keyboard grabs where mutter's `xwayland-allow-grabs` lets them through.
* "Fullscreen windowed" counts only when mutter treats the window as fullscreen (`is_fullscreen()`: the app asked
  for fullscreen, or mutter's own X11 legacy-fullscreen handling). A Wayland window that only sizes itself to the
  monitor is kept to the work area (scenario part I), so the taskbar stays, as before. Not tested: an Xwayland
  borderless game (the harness has no X11 client).
* A monitor-sized override-redirect window (some X11 games, screensavers) makes the monitor fullscreen but nothing
  can be stacked above it: the taskbar stays hidden as before, and Start opens below it as before.
* Esc in another taskbar flyout opened over a fullscreen app (search, quick settings, calendar, notifications): the
  panel keeps the keyboard as it always did and the taskbar stays until you click the app or its button. Not
  returned on purpose: those flyouts launch apps, and an activation of the fullscreen app after the launch could
  make mutter's focus-stealing prevention map the new app behind it. The task view only focuses existing windows.
* Out of fullscreen, Esc in the task view still leaves the keyboard on the panel (old behaviour, probe step 13);
  a candidate for a later round.
* While the taskbar shows over a fullscreen app no strut is reserved (unchanged), so a window opened then may lie
  under the taskbar until fullscreen ends.
* The rule is for the primary monitor (the taskbar's). Multi-monitor is not tested (one virtual monitor).
* A cold Start launch slower than 2 s (Start is normally preloaded) hides the taskbar after 2 s and shows it again
  when Start maps.
* The fallback St taskbar (only while win11-panel is missing) keeps GNOME's trackFullscreen behaviour.

## Left for the coordinator

* Commit (nothing is committed; `git status`: `M extension/panelBridge.js`, new `notes/claude/round5/`,
  `tests/gnome/scenarios/r5-fullscreen.py`, its outputs `r5-fullscreen.{out,en.out,before.out,probe1,regress}/`;
  the PNGs are excluded by `.git/info/exclude`).
* The extension changed, so the package needs a rebuild of the extension part only (no Rust change; the 12
  binaries of 7efc1e61 were used) and Robin's live test needs a re-login after installing.
* Follow-ups: Esc in the task view out of fullscreen (keyboard stays on the panel), and a live check with an
  Xwayland borderless game.
