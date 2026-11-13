# GE0c: private headless GNOME Shell harness (2026-11-05)

G = `/home/robin/Documents/Codex/2026-11-01/chat/outputs/win11-gnome`, branch `claude/gnome-english-update`
(HEAD 7fc86a6d = 3b898e6 plus `notes/claude/`). The input is the GE0b debug build in `target/debug`.

* Nothing was committed and no product code was changed.
* The only change outside my files is one line appended to `.git/info/exclude` (`tests/gnome/run/`), as the
  task allowed.
* Git status at the start matched the GE0b report. During my run another agent added
  `notes/claude/windows-update-gnome-critique.md` (GE0d critic). I left it alone.

## 1. What I built

| Path | What |
|---|---|
| `tests/gnome/harness.py` | The harness. Commands: `guard-selftest`, `launch run …`, `launch probe`, plus the internal stages `run`/`_inner`. |
| `tests/gnome/README.md` | How to run it, what it guards, the layout and the known differences from the live session. |
| `tests/gnome/baseline/nl/` | 15 PNGs and `results.json`. `logs/` holds `shell.log`, `harness.log`, `install-verify.json`, `guard-calls.log` and `memory.tsv`. |
| `tests/gnome/baseline/en/` | The same, 15 PNGs. |
| `tests/gnome/baseline/switch/` | 4 PNGs of the live nl→en switch with Settings (Taal en regio) and Start open. |
| `tests/gnome/baseline/guard-selftest.json` | 56 cases, 56 passed. |
| `tests/gnome/findings/minimal-seed-live-switch/` | 4 PNGs and `results.json` for finding F3 below. |
| `tests/gnome/run/` | All run state (about 750 MB), rebuilt on every run, ignored through `.git/info/exclude`. |

Surfaces:

* panel (taskbar strip), desktop, Start, Verkenner;
* Instellingen: home, system, display, power, about, personalization, language, update;
* Taakbeheer, Alles, Schijven;
* the switch: before, after with Start open, after with Settings only, the panel after.

## 2. Reproduce

```bash
cd /home/robin/Documents/Codex/2026-11-01/chat/outputs/win11-gnome
python3 -B tests/gnome/harness.py guard-selftest                                   # about 3 s, no GUI process
grep MemAvailable /proc/meminfo                                                    # run refuses at 2 GB or less
flock -w 3600 $PWD/.heavy.lock python3 -B tests/gnome/harness.py launch run --lang nl --switch   # 154 s
flock -w 3600 $PWD/.heavy.lock python3 -B tests/gnome/harness.py launch run --lang en            # 144 s
python3 -B tests/gnome/harness.py launch run --lang nl --switch --seed minimal --out tests/gnome/run/f3   # F3 repro
```

`launch` sees the `flock` ancestor, so it does not lock twice. Without the wrapper, it takes `.heavy.lock` itself
(waiting up to 3600 s). The lock is held by `run` inside the sandbox until the session has ended.

## 3. How Codex did it, and what I did not copy

I read `W/desktop-bridge-test-0.7.0/run-combined.py`, `W/settings-native-test/run.py`, `W/gnome-test/run.py`, their
shell logs, and extension-packaging.md section (d).

**Kept:**

* private `HOME` and XDG directories, keyfile GSettings;
* `install.py` into `WIN11_INSTALL_PREFIX` with `WIN11_BUILD_DIR`;
* `gnome-shell --headless --virtual-monitor --wayland-display --no-x11` under `dbus-run-session`;
* `org.gnome.Shell.Eval`;
* `WIN11_DEV_SCRIPT`/`WIN11_DEV_COMMAND`/`WIN11_DEV_REPORT`;
* `org.gnome.Shell.Screenshot` on the private bus;
* the language set in the private `settings.json`.

**Not copied**, because each one touched or could touch the live session:

* **`gnome-test/run.py` (11-01):**
  * it kept the real HOME;
  * it put `~/.local/share` into `XDG_DATA_DIRS`;
  * it pointed `WIN11_REFERENCE_SETTINGS` at the live `~/.config/win11shell/settings.json`;
  * it pointed `WIN11_REFERENCE_INSTANCE` at the live `/run/user/1000/win11shell/instance.json`, so the private
    apps called Robin's live backend;
  * it copied the live `user-dirs.dirs`;
  * it used live wallpaper paths;
  * it wrote preview PNGs into G.
* **`run-combined.py` and `settings-native-test/run.py`:**
  * They start from `os.environ.copy()` and pop only a few keys. So PATH kept `~/.local/bin` (the live
    `win11-*`), and `SSH_AUTH_SOCK`, `XAUTHORITY` and `GPG_AGENT_INFO` stayed.
  * They set no `NO_AT_BRIDGE`.
  * They use dbus-run-session's default config, whose standard service directories auto-activated e.g. the IBus
    portal.
  * They use the real system bus. The shell logged "Will monitor session 42" (Robin's logind session),
    "Registering session/display with GDM" and an AuthenticationAgent registration that failed only because the
    live shell holds it.
  * They had network access: gnome-shell's extension update check goes to extensions.gnome.org.
  * They kept their state in `/tmp/win11-desktop-test` and `/tmp/win11-gnome-build-0.6.2`.
* **DING (`run-combined.py`).** It enabled DING from a private copy, because DING's stale-process matcher would
  otherwise kill Robin's DING. I do not load DING at all: the session mode is `user`, not `ubuntu`, and the
  keyfile enables only our UUID.
* **Codex's live deploy scripts** (the table in extension-packaging (d)): not used.

**New in GE0c** (README "What it guards"):

* A **bubblewrap sandbox**:
  * read-only root and real HOME (only `tests/gnome/` writable);
  * private tmpfs on `/tmp`, `/var/tmp`, `/run/user` and `/run/dbus`;
  * a minimal `/dev` (GPU nodes only);
  * its own PID namespace (no process outside can be signalled, everything dies with the sandbox);
  * its own network namespace (loopback only).
* A **complete replacement environment**.
* A **private empty system bus** as well as the session bus; neither has service directories.
* `--unsafe-mode` on the command line instead of patching `extension.js`, so the private extension copy is
  byte-identical to `extension/`.
* PATH stubs.
* An strace proof for the install.
* A memory sampler with an abort below 1.5 GB.

## 4. `scripts/install.py`: what it writes

I read `install.py`, `icon_bundle.py` and `file_portal.py` before the first run.

* **Path sources.** Every write location comes from `WIN11_INSTALL_PREFIX`, `XDG_DATA_HOME` or `XDG_CONFIG_HOME`.
  `Path.home()` is used only as the default when one of these is unset, and the guard requires all three.
* **What it writes:**
  * `prefix/bin/` (12 binaries, through `.name.new` and rename);
  * `prefix/share/win11-gnome/` (staged in `prefix/share/.win11-gnome-install-*`);
  * `data/gnome-shell/extensions/<uuid>/` (and `glib-compile-schemas`);
  * `data/applications/` (10 entries, plus `mimeinfo.cache` from `update-desktop-database`);
  * `data/icons/Win11Shell{,-dark}` (symlinks);
  * `data/xdg-desktop-portal/portals/win11shell.portal`;
  * `data/dbus-1/services/…win11shell.service`;
  * `config/systemd/user/app-win11-file-portal.service`;
  * `config/xdg-desktop-portal/gnome-portals.conf` (only the FileChooser key).
* **Reads only:**
  * GSettings `org.gnome.desktop.interface icon-theme` (keyfile, read-only);
  * `/etc/xdg`, `/etc`, `XDG_DATA_DIRS` and `~/.icons` (private HOME).
* **Never** writes GSettings and never enables the extension.
* **strace proof** (`strace -f -y -e trace=%file`, every run): 87,844 mutating syscall paths, **0 outside
  `tests/gnome/run/`**.
* **Install check:**
  * all 12 binaries are byte-identical to `target/debug`;
  * all 9 extension files plus the schema match `extension/`;
  * there are 10 desktop entries, and all `Exec=` lines point into the prefix.
* **Time:** 70 s per install, mostly strace overhead on the 35k-entry icon tree.

## 5. Guard and self-test

`python3 -B tests/gnome/harness.py guard-selftest` gives **56/56 passed** (`baseline/guard-selftest.json`).

**Required refusals.** Each one is shown in-process, as a real process outside the sandbox, and inside the real
bwrap sandbox with only that variable changed:

| Condition | Code | Variants tested |
|---|---|---|
| HOME is the real home | `home-real` | also with a trailing `/` |
| `XDG_RUNTIME_DIR=/run/user/1000` | `runtime-live` | — |
| Session bus points into `/run/user/1000` | `session-bus-live` | plain, with `guid=`, `%2f`-escaped, second entry of a list |
| `DISPLAY` set | `display-set` | also empty |
| `WAYLAND_DISPLAY` set | `wayland-display-set` | — |
| `GSETTINGS_BACKEND` not keyfile | `gsettings-not-keyfile` | unset, `dconf` |

**Extra refusals:**

* the session bus is abstract, tcp or unset;
* the system bus is the real one;
* `NO_AT_BRIDGE` or `GTK_A11Y` is missing;
* `~/.local/bin` is on PATH;
* the live `XDG_CONFIG_HOME` or `XDG_DATA_HOME` is used;
* `WIN11_REFERENCE_INSTANCE` is set;
* `GNOME_SHELL_SESSION_MODE=ubuntu` is set;
* HOME is outside the run dir;
* in the inner stage, the bus is not `rt/bus`.

**Process level:**

* The caller's own live shell environment is refused with every one of the codes above, plus `not-sandboxed-*`.
* The private environment outside bwrap is refused with only `not-sandboxed-*` codes: the home is writable, the
  live sockets are visible, the network is not loopback-only, PID 1 is systemd.
* The private environment inside bwrap is accepted.

## 6. Results

### 6.1 Runs (final code, 14:17-14:22)

| Run | Surfaces | PNGs | Wall time | Failures | `lang` attribute | Blank WebViews |
|---|---|---|---|---|---|---|
| nl + switch | 18 (15 + 3 switch) | 19 | 154 s | 0 | all `nl` before the switch, all `en` after | 0 |
| en | 15 | 15 | 144 s | 0 | all `en` | 0 |

* **Shell.** GNOME Shell 50.1 in mode `user`, with `unsafe_mode` true. It answered Eval about 4 s after start.
* **Extension.** State ACTIVE, no error, and the copy is unpatched.
* **WebViews.** Panel, desktop and Start reported `dataset.ready` without help.
* **Leftovers.** No process was left over in either run (`leftoverProcessesAfterInner: []`). Afterwards
  `pgrep -af "tests/gnome/run|w11-wl|harness.py"` on the host found nothing, and the real `/tmp` holds no
  `win11-*` entry.
* **Stub calls (nl run):**
  * 67 × `wpctl get-volume` and 21 × `pw-dump --monitor` (the panel's volume tray, which retries about every second
    without PipeWire);
  * 10 × `systemctl --user list-units` (Taakbeheer);
  * 2 × `apt list --upgradable` (the Update page). The current update page uses `apt list`, not PackageKit
    (relevant for Windows Update).

### 6.2 Memory of the whole private session

The sampler reads `/proc/*/smaps_rollup` every 0.5 s and covers every process in the sandbox's PID namespace.

| | nl + switch | en |
|---|---|---|
| Peak PSS, whole sandbox | **1,088 MiB** | **1,081 MiB** |
| Peak PSS without the harness's own Python and bwrap | 1,003 MiB | 996 MiB |
| Sum of RSS at the peak (shared pages counted more than once) | 2.53 GB | 2.53 GB |
| Processes at the peak | 33 | 33 |
| Lowest MemAvailable during the run (started at about 4.9 GB) | 3.76 GiB | 3.69 GiB |

**At the peak** (about 31 s, Settings open next to panel, desktop and Start):

| Process | PSS |
|---|---|
| 5 × WebKitWebProcess | 363 MiB |
| gnome-shell | 258 MiB |
| harness Python (2 processes) | 84 MiB |
| win11-app (Settings) | 70 MiB |
| win11-panel | 66 MiB |
| win11-start | 63 MiB |
| win11-desktop | 61 MiB |
| 5 × WebKitNetworkProcess | 55 MiB |
| 4 × glycin-svg | 27 MiB |
| ibus (spawned by gnome-shell) | about 20 MiB |

**Cold cache.** The first smoke run had a cold GStreamer registry and peaked at 1,281 MiB, because three
`gst-plugin-scanner` processes ran at once (531 MiB PSS). The harness now builds the registry once before the
session (2.6 s), as the live session already has it.

### 6.3 What I looked at (Read on the PNGs)

* **nl desktop.** Bloom wallpaper. Icons: Deze pc, Prullenbak, Folder, alpha.txt, beta.txt. Centred taskbar with
  Start, Zoeken, task view and the Verkenner, Firefox and Instellingen pins. Tray: globe and clock "14:13 /
  05-11-2026". It looks right.
* **nl Start.**
  * Labels: "Zoek naar apps, instellingen en documenten", "Vastgemaakt", "Alle apps", "Aanbevolen", and the
    empty-state text.
  * App names: Verkenner, Instellingen, Taakbeheer, Alles, Teksteditor, Rekenmachine, Afbeeldinge…, Agenda,
    Klokken.
  * User "robin".
* **nl Instellingen:**
  * Startpagina: Gigabyte B550, "Netwerk: Niet verbonden", "Geen Bluetooth gevonden" (no system services).
  * Beeldscherm: "Niet beschikbaar", plus the D-G2 texts (F4).
  * Energie en batterij: Energiestand niet beschikbaar.
  * Taal en regio: Nederlands, and the Notaties sample "zondag 25 oktober 2026", "1.234.567,89".
  * Update en herstel: "Kan niet controleren op updates", Versie 0.7.0.
* **nl Verkenner.** A tab "robin" with the 8 fixture folders. Dutch sidebar: Bureaublad, Documenten,
  Afbeeldingen, Muziek, Video's, Deze pc with the real mounts, Prullenbak.
* **nl Taakbeheer.** Samenvatting: "Je pc werkt prima", Processor 7 %, Geheugen 74 %, RTX 3080, Netwerk 0 bps.
* **nl Alles.** "Vind alles, meteen" with the quick-search chips.
* **nl Schijven.** "Schijfbeheer werkt niet: UDisks is op deze computer niet beschikbaar" (no UDisks on the
  private bus).
* **en.** Every surface is English: "Pinned", "All apps", "Recommended", "Search for apps, settings and
  documents", "This PC", "Recycle Bin", "Home – Settings", "Update & recovery", "Disk management doesn't work:
  UDisks isn't available on this computer", date "05/11/2026".
* **The Dutch-word scan** (`tests/i18n/dutch-words.txt` against `innerText`) found nothing in any fresh en surface.

**Nothing was broken in the final runs.** Every WebView painted (70-2,000 distinct colours per surface region; the
blank flag fires at 4 or fewer), and the taskbar was present in every shot.

Two things in the first runs were harness artifacts, and both are fixed:

* **"Is ready" banners.** Apps that I started without an activation token mapped unfocused, and GNOME posted
  "“File Explorer” is ready" banners over the bottom-right corner. The harness now records the banner, focuses the
  window and dismisses it.
* **"Screen Lock disabled".** GNOME posted this notice because there is no GDM on the private bus, and it showed
  as the "1" badge on the taskbar. It is now dismissed and recorded.

### 6.4 Findings about the product at 3b898e6

* **F1. The live switch leaves Start partly in Dutch.**
  * **What stays Dutch.** After nl→en with Start open, the headings "Vastgemaakt", "Alle apps" and "Aanbevolen" stay
    Dutch (`baseline/switch/switch-2-*`; the Dutch-word scan reports `aanbevolen`, `alle`, `vastgemaakt`). Fresh
    en shows "Pinned", "All apps" and "Recommended".
  * **Search placeholder.** After the switch it reads "Type here to search". Fresh en uses "Search for apps,
    settings and documents", so the live path uses a different string.
  * **What does switch.** Everything else switches: Settings in 0.21 s, Start's `lang` in 0.31 s, the panel in
    0.41 s, the desktop in 0.51 s and the Settings page title in 0.62 s. Pins and theme stay.
  * **Owner:** GE1a (Start re-label on `settings.changed`).
* **F2. Native window titles are English in both languages.**
  * **Evidence.** The Meta window titles are "Settings", "File Explorer", "Task Manager", "Everything", "Disks",
    "Start", "Win11 Desktop" and "Win11 Taskbar", while `document.title` is Dutch ("Startpagina –
    Instellingen", "robin - Verkenner"). GNOME's own "“Settings” is ready" banner and the overview and Alt+Tab
    show the native title.
  * **Alles.** Its native title is "Everything" in nl and in en, which goes against D-G5.
  * **Owner:** GE1a/GE1b (D-G6). The harness records `windowsAtShot` and `attentionNotifications` so the fix can be
    checked.
* **F3. Rewriting a partial `settings.json` live drops all defaults.**
  * **Cause.** `host/src/settings.rs:327-331` merges defaults only on the first read; every later reload takes
    the file as written.
  * **Effect.** With `{"language":"nl"}` as the file, rewriting it to `"en"` while the apps ran turned the theme
    from dark to light and removed the Verkenner and Firefox pins until restart
    (`findings/minimal-seed-live-switch/`).
  * **Harness seed.** The harness now seeds the complete `host/src/settings-defaults.json`, the shape of a file the
    host has saved.
  * **Risk.** Any writer other than the host (GE1c installer `--lang`, the extension, scripts) must write a
    complete document. Otherwise the reload has to merge defaults, which is a product change.
* **F4. D-G2 texts confirmed on screen.**
  * Display: "Nachtlampje werkt alleen in de Win11 Shell-sessie."
  * Display: "Beeldschermen instellen kan alleen in de Win11 Shell-sessie. Gebruik hier de app Instellingen van
    Ubuntu."
* **F5. Extension teardown warning.** When the desktop client goes away, two Gjs-CRITICAL "MetaWindowActorWayland
  has been already disposed" appear: `desktopBridge.js:90` (`unmanaged`) calls `_unwatchWindow`, which
  disconnects actor signals on a disposed actor (`:236`). The harness hits this at shutdown, and a desktop crash
  would hit it too. It is log noise only.
* **F6. Volume polling without PipeWire.** Without PipeWire the panel calls `wpctl` and restarts `pw-dump --monitor`
  about once per second, with no backoff. Low priority.

### 6.5 Known differences from the live session (also in README)

* **Shell theme.** The GNOME chrome uses the stock theme, not Yaru (mode `user`).
* **No services.** Network, Bluetooth, UPower, logind, PackageKit, UDisks and audio are all absent (empty private
  buses).
* **Sandbox-only views.** Taakbeheer lists only the sandbox's processes. "Deze pc" lists the real mounts read-only.
* **GPU.** The GPU render node is used (`nvidia-drm`, no mode setting), as in Codex's runs.
* **Clock.** The clock and date in every shot differ per run. A pixel diff must mask the taskbar clock and the
  Settings Notaties sample.

## 7. Not done

* No real input (Clutter virtual devices). No panel flyouts (quick settings, notification centre, calendar, task
  view), context menus, Explorer dialogs or Start "Alle apps".
* 8 of the 34 Settings pages; no Standaard-apps, no Installer.
* No extension label checks through Eval (search button, Ctrl+Alt+Tab names, the "missing app" notification) and
  no fallback St taskbar.
* No DING interplay (it would need Codex's private DING copy). No scale 2 or second monitor.
* No pixel-diff tool. `results.json` text is the comparison oracle for now.
* Debug build only (GE0b); no release build.
* I ran one `stat` (metadata only) on `~/.config/win11-gnome` to see that its mtime (13:07:55) predates my first
  run (14:00). Nothing else under the live paths was read or touched.

## 8. For the coordinator

1. **Adopt the bwrap sandbox** (`harness.py launch`) as the only way to run GUI tests. It is stronger than the
   plan's guard list: the real HOME is read-only, the live sockets are invisible, and there are PID and network
   namespaces.
   * **Requirement:** an unprivileged user namespace through bwrap's AppArmor profile, which works on this PC.
   * **GE0b.** Its `/tmp` exception could go away by running cargo test inside the same sandbox.
2. **No `/tmp` exception needed here.** The product's socket fallback to `/tmp/win11-events-…`
   (`host/src/files/mod.rs:435-449`) triggers because the repo `XDG_RUNTIME_DIR` is 65 characters or more, but it
   lands in the sandbox's private `/tmp`.
3. **Commit the PNGs?** There are 38 (nl 15, en 15, switch 4, findings 4), about 16 MB. The alternative is to commit
   only `results.json` and the logs and keep the PNGs out of git.
4. **Assign F1-F3:**
   * F1 → GE1a;
   * F2 → GE1a/GE1b (D-G6; Alles' native title "Everything" vs D-G5);
   * F3 → GE1c (every `settings.json` writer writes the full document), or a decision to merge defaults on reload
     (product change).
5. **Commit list:**
   * `tests/gnome/harness.py`
   * `tests/gnome/README.md`
   * `tests/gnome/baseline/` (with or without PNGs, per item 3)
   * `tests/gnome/findings/`
   * `notes/claude/round0/ge0c.md`

   Do not commit `tests/gnome/run/`, which is already excluded.
6. **Cost.** Each run is about 2.5 min and about 1.1 GB PSS (MemAvailable falls by about 1.1-1.2 GB). It runs
   under `.heavy.lock`, so it cannot overlap cargo.
