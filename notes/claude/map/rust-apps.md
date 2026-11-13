# Map: Rust host outside `services/` (files, apps, portal, launch, window, window_state, picker, gnome, images, transfers, wallpaper, process_scope, lib, bin)

Read-only survey of G (`/home/robin/Documents/Codex/2026-11-01/chat/outputs/win11-gnome`, branch `claude/gnome-english-update`, identical to `codex-baseline` 3b898e6 for `host/`). Paths below are relative to `G/host/src` unless they start with `G/` or `H/`. No builds, no binaries started, no settings changed. Two files under `~/.config/win11-gnome` were read (read-only), as the rules allow.

## Summary

- **English already exists at the core.** `settings-defaults.json:3` is `"language": "nl"`, and `"en"` is a valid value. `Files::context()` (`files/mod.rs:142-170`) and `ServiceContext::language()` (`services/mod.rs:62-69`) both give back `nl`/`en`. Several modules use it already: places, new-item names, copy suffix, type names, desktop-entry names, properties, the app catalog, default apps and the per-type labels. In total that is **23 inline nl/en pairs** in Rust plus bilingual data files: 47 MIME types in `files/types.json`, and in `apps/catalog-data.json` 7 own apps × name/generic/comment plus 1 twin. When the language changes, the app catalog rebuilds itself and emits `apps.changed` (`apps/mod.rs:221-244`).
- **Most host messages are still Dutch only.** In this area there are about **231 Dutch-only literals the user can see**: API errors shown in Explorer, the picker, Settings and so on. 133 are in `files/**`, 28 in `apps/**`, 14 in `picker.rs`, 15 in `process_scope.rs`, 13 in `transfers.rs`, 10 in `gnome.rs`, 7 in `wallpaper.rs` and 11 in `launch.rs`/`bin/launch.rs`/`lib.rs`. **About 112 of them (48%) appear word for word in H's `i18n/host/*.tsv`, together with an English translation**, so H's catalogs can be reused.
- **Window titles are fixed English strings and never follow the UI** (`window.rs:187-204`). There are 11 of them: "File Explorer", "Settings", "Task Manager", "Everything", "Disks", "Default apps", "File selection", "App installer", "Start", "Win11 Taskbar" and "Win11 Desktop". No `document.title` → GTK title sync exists, unlike H's `appwindow.py:534/1110`. The UI's `setTitle` IPC is skipped in native windows (`vendor/ui/js/apps/appframe.js:255`). So the Win11 taskbar previews and Task View (`taskbar.js:432`, `taskview.js:156`) and GNOME probably show English titles even in Dutch. This comes from reading the code; I did not check it live.
- **No Rust GTK widget has a label.** The only native strings are window titles. There are no message dialogs and no file-operation conflict dialogs: copy/move never overwrite and pick unique names instead ("X (2)", "X - kopie"/"X - Copy").
- **No binary passes a language to its WebView.** The URL carries only `token`, `surface`, `app` and arguments (`lib.rs:373-425`, `picker.rs:89-97`, `portal.rs:452-455`). The UI takes the language from settings (`vendor/ui/js/core/theme.js:319`). **Passing `?lang=` would be wrong:** it *forces* the language (`theme.js:50`, `i18n.js:14-17`) and breaks switching at runtime.
- **Window memory exists only for our own 7 apps:** explorer, settings, taskmgr, alles, disks, defaultapps and installer (`window_state.rs:21-26`). It is stored in `~/.config/win11-gnome/window-state.json` (0600, flock, atomic rename). It remembers the normal rect (x, y, w, h), whether the window was maximized, and Explorer's tabs. On reopen, the GTK default size is applied first and the position is set afterwards by the extension (`windows.hostRestore`). Robin's file holds entries for explorer, settings and taskmgr, so it works. The term "windowGeometry" does not occur anywhere in G or H.
- **What "every app" still needs:**
  - nothing covers third-party apps; that would need work in the GNOME extension;
  - own apps only save when the process exits cleanly (no SIGTERM handler, so state is lost at logout);
  - a second window opens exactly on top of the first (no cascade);
  - every open own-app window polls GNOME Shell over D-Bus every 500 ms (`window_state.rs:318,345`).
- **English from the extension reaches the UI:** `gnome::call_api` (`gnome.rs:17-27`) passes the extension's English failure messages through (29 `failure()` calls in `G/extension/panelBridge.js`). In Dutch mode these are mixed-language messages already today.
- **Probable catalog bug (from code reading):** if the shell language is `en` on a *Dutch* system locale, third-party app names fall back to GIO's localized (Dutch) name (`apps/catalog.rs:300-308`). It does not show on this PC because the session runs `LANG=en_US.UTF-8`, but it may on the NucBox.

## Findings

### Method

I wrote a Rust string-literal extractor that handles raw strings, byte strings, comments and char literals. It skips `#[cfg(test)] mod … { }` blocks by brace matching, plus `portal/tests.rs` and `launch_context/portal/tests.rs`. It found 503 non-test literals with letters in them. A word-list heuristic sorted them into Dutch, English, code (JS adapters) and neutral (identifiers, keyfile keys, MIME types, HTTP headers). I then reviewed every flagged literal by hand. I removed false positives such as the keyfile key `"Type"` and the XDG fallback directory name `"Downloads"`, and added Dutch strings that have no marker words (`"gebruiker"`, `"{}-bestand"`, `"kopie"`). Literals that sit on the line after `eprintln!(` are counted as log. The extractor and its output are scratch files and not part of G.

### (a) User-visible strings per file

Legend:
- **NL**: a Dutch-only string the user can see (API error, label, picker error);
- **Pair**: already nl/en and chosen by the language setting;
- **EN-vis**: English the user can see;
- **EN-dev**: English that only a caller or developer sees (D-Bus or portal errors, internal io errors);
- **Log**: stderr or CLI output (NL/EN).

| File | NL | Pair | EN-vis | EN-dev | Log NL/EN | Notes |
|---|---:|---:|---:|---:|---|---|
| files/actions.rs | 30 | 0 | 0 | 0 | 0/0 | open/openWith/launcher trust/terminal; `:348` names the UI button "Starten toestaan" |
| files/archives.rs | 26 | 0 | 0 | 6 | 0/0 | ~10 are ZIP-internal ("Ongeldige ZIP-index."); 6× `"unsafe archive link"` are internal, mapped to Dutch `unsafe_link()` `:55,:1423` |
| files/archive_ffi.rs | 3 | 0 | 0 | 0 | 0/0 | missing libarchive |
| files/clipboard.rs | 8 | 0 | 0 | 0 | 0/0 | |
| files/dry.rs | 3 | 3 | 0 | 0 | 0/0 | dry-run copy of mutate's new-item names `:74-79` |
| files/mod.rs | 26 | 0 | 0 | 0 | 0/0 | `validate_name` `:577-607`, `io_error` `:609-641` (shared by everything) |
| files/mutate.rs | 8 | 4 | 0 | 0 | 0/0 | pairs `:68-73` (Nieuwe map/New folder, Nieuw tekstdocument.txt/New Text Document.txt…), copy suffix `:395-399` |
| files/places.rs | 1 | 8 | 0 | 0 | 0/0 | pairs `:18-82` (Bureaublad/Desktop … Prullenbak/Recycle Bin, Systeemschijf/System drive); `"gebruiker"` fallback `:155` |
| files/properties.rs | 5 | 2 | 0 | 0 | 0/0 | pairs `:158-168`; `"{} items"` `:182` works in both languages |
| files/read.rs | 2 | 4 | 0 | 0 | 0/0 | pairs: This PC/Deze pc `:492-494`, type_name `:606-641`; launcher-names.json |
| files/search.rs | 3 | 0 | 0 | 0 | 0/0 | |
| files/trash.rs | 14 | 0 | 0 | 0 | 0/0 | `:1230` "{failed} item(s) konden niet…" |
| files/watch.rs | 4 | 0 | 0 | 0 | 0/0 | |
| apps/defaults.rs | 12 | 1 | 0 | 0 | 0/0 | pair `:171` Verkenner/File Explorer |
| apps/mod.rs | 8 | 0 | 0 | 0 | 0/0 | |
| apps/pertype.rs | 7 | 1 | 0 | 0 | 0/0 | pair `:93-98` "{s}-koppelingen"/"{s} links" |
| apps/recent.rs | 1 | 0 | 0 | 0 | 0/0 | |
| apps/catalog.rs | 0 | 0 | 0 | 0 | 0/0 | text comes from catalog-data.json and desktop files (see below) |
| picker.rs | 14 | 0 | 0 | 0 | 0/0 | shown in the picker via `win11:picker-error` (`window.rs:403`, `explorer/picker.js:87`) |
| transfers.rs | 13 | 0 | 0 | 0 | 0/0 | tab drag between Explorer windows (`runtime.rs:162-167` → ApiError `tab_transfer`) |
| wallpaper.rs | 7 | 0 | 0 | 0 | 0/0 | |
| gnome.rs | 10 | 0 | 0 | 0 | 0/0 | mostly internal (input regions `:223-260`); `:25` is the fallback when an extension error has no message |
| process_scope.rs | 15 | 0 | 0 | 0 | 2/0 | most are internal systemd/UTF-8 checks; "Programma niet gevonden", "Het programma kon niet worden gestart." and "…niet op tijd gestart." (`:212,:425,:428`) can bubble up from launches |
| launch.rs | 3 | 0 | 0 | 0 | 0/0 | `command()` `:29-35` |
| bin/launch.rs | 5 | 0 | 0 | 0 | 1/0 | errors sent back through the ack socket as JSON |
| lib.rs | 3 | 0 | 0 | 0 | 2/0 | only ever printed to stderr through `run()` `:268-271` |
| window.rs | 0 | 0 | 11 | 0 | 3/0 | titles `:187-204`; 9 belong to real app windows, 2 to hidden surfaces |
| window_state.rs | 0 | 0 | 0 | 4 | 0/1 | io errors are never shown |
| portal.rs | 0 | 0 | 0 | 14 | 0/6 | D-Bus errors to the calling app; hidden parent title `:224` |
| portal/external.rs | 0 | 0 | 0 | 10 | 0/0 | |
| portal/model.rs | 0 | 0 | 0 | 16 | 0/0 | "Invalid FileChooser {what}" |
| bin/app.rs | 0 | 0 | 0 | 0 | 3/0 | CLI usage; it lists 4 apps but also accepts defaultapps/installer `:22-23` |
| bin/file_portal.rs | 0 | 0 | 0 | 0 | 0/1 | |
| images.rs, images/* | 0 | 0 | 0 | 0 | 0/0 | only HTTP bodies "Forbidden"/"Not Found" for `<img>`, never shown |
| launch_context.rs, launch_context/portal.rs, app_params.rs | 0 | 0 | 0 | 0 | 0/0 | neutral |
| bin/{start,panel,desktop,explorer,installer,defaultapps,*_worker}.rs | 0 | 0 | 0 | 0 | 0/0 | one-line `run(kind)` |
| **Total** | **231** | **23** | **11** | **50** | **11/8** | about 103 further literals are neutral (keyfile keys, MIME types, headers, systemd properties) and 14 are JS adapter code with no visible text |

Data files (all `include_str!`):

| File | Bilingual | NL-only | Notes |
|---|---|---|---|
| `apps/catalog-data.json` | `ownApps`: 7 × {name, generic, comment} nl/en; `twins`: 1 | `nlNames`: 57, `nlComments`: 42 | Dutch overrides for third-party apps, used only when `lang == "nl"` (`apps/catalog.rs:300-330`). English falls back to the desktop file, which is correct. `keywords` mix both languages on purpose. |
| `files/types.json` | 47 MIME → [nl, en] | – | used by `read::type_name` `:606` |
| `files/launcher-names.json` | `SAME_NAME_AS_OURS`: 1 | `NL_NAMES`: 57, identical to `nlNames` | `read.rs:470-482`; a duplicate data set (risk of drift) |
| `api-methods.json` | – | – | 172 method docs; developer-facing, some quote Dutch UI labels |
| `settings-defaults.json` | – | – | `language: "nl"`; carries no labels |

Other notes on (a):
- **Error envelope.** `ApiError {code, message, data}` (`files/mod.rs:28-50`). The UI shows `e.message` directly in about 111 places (for example `explorer/main.js:825,911,1007`). Codes are stable, but the same code is used for many different Dutch messages (`bad_args`, `not_found`, `exists`), so translating by code alone in the UI would lose detail. The host has to render the text in the caller's language. `io_error` also attaches the OS error text, which libc localizes to `LANG`, as `data` on `failed`.
- **Generated `.desktop` entries are not in Rust.** They come from `G/scripts/install.py:111-131`: Dutch-only `Name=` ("Win11 Verkenner", "Win11 Taakbeheer"…) plus a Dutch `Comment=`, with no `Name[en]`. That file is outside this area, but these names show in GNOME's overview.
- **Installer texts.** `bin/installer.rs` only calls `run("installer")`. Its texts live in `services/software/**` (another agent's area) and `vendor/ui`.
- **Portal and file chooser.** The caller sends its own `title`/`accept_label`/filter names (`portal/model.rs:85-113,241-242`), already in the calling app's language. Our own texts are the 14 picker errors plus the GTK title "File selection".

### (b) How each binary creates its WebView

Common path: `lib.rs:259-488` `run(kind)`, then `window.rs:151-843` `build()`.

1. `process_scope::ensure_native(kind)` (`process_scope.rs:311-331`) re-runs the program inside a transient systemd unit. Shell surfaces go into `app-win11shell.slice` (MemoryMax 1000 MiB).
2. The app id is set per kind (`lib.rs:278-289`); unknown kinds become `io.github.win11shell.Explorer`.
3. `Runtime::new` starts the per-process backend and HTTP server (`runtime.rs:40-60`). The config directory is `$WIN11_GNOME_CONFIG_DIR` or `~/.config/win11-gnome`.
4. `WindowState::open` (`lib.rs:358`) loads the saved window state.
5. URL: `http://127.0.0.1:<port>/ui/index.html?token=…` plus `surface=shell` (start, panel), `surface=desktop`, or `surface=app&app=<kind>`. Then from argv: `--tab-ticket X` → `tabTicket`, `--params JSON` → every key as a query pair, otherwise the first positional argument → `path` and the rest → `tabs` (JSON) (`lib.rs:373-425`). **No `lang`.**
6. Wry builder (`window.rs:474-542`):
   - one `WebContext` shared per process (`window.rs:14-18`), unless `WIN11_PRIVATE_WEB_CONTEXT=1`;
   - `with_transparent(true)`;
   - init script `NATIVE_BRIDGE` (`lib.rs:138-140`) plus `windowId`, `preloadStart`, `explorerSession` (saved tabs) and `nativeBackdrop` (true for panel and the 7 apps);
   - one adapter per surface: `START_ADAPTER` `lib.rs:141-226`, `gnome::PANEL_ADAPTER`/`DESKTOP_ADAPTER` (`gnome.rs:285-396`, no visible text);
   - optional `WIN11_DEV_SCRIPT`;
   - IPC handler, and a drag-drop handler for the installer only (`:490-516`);
   - navigation limited to the local origin; external http(s) goes to `open_external_uri` (`:533-542`).
7. After building:
   - cache model `DocumentViewer` (`:553-567`);
   - hardware acceleration `Never`, unless `WIN11SHELL_GPU=safe|env` and no `no-gpu` flag file exists (`:582-594`);
   - a `win11` script-message handler (`:625-638`);
   - for desktop, panel and start, a web-process crash quits the app (`:568-581`).

   The host sets no WebKit language (no `set_preferred_languages`), so `navigator.language` and WebKit's own UI strings follow the process `LANG`, which is `en_US.UTF-8` on this PC.

| Binary | Kind / app id | Uniqueness | URL extras | Special |
|---|---|---|---|---|
| win11-start | start / `.Start` | unique, registers first; a remote launch calls `activate()`, which toggles | `surface=shell` | `--preload` (spawned by the panel with PDEATHSIG `lib.rs:520-558`), 642×726, realized but not mapped, resident hold if the GNOME bridge is present |
| win11-panel | panel / `.Panel` | unique | `surface=shell` | Dock type hint, no focus, input regions, size from `panel.geometry`, prewarms Start, actions `panel-taskview`/`panel-search` |
| win11-desktop | desktop / `.Desktop` | unique | `surface=desktop` | Desktop type hint, sticky, keep-below, size from `desktop.geometry` |
| win11-explorer | explorer / `.Explorer` | NON_UNIQUE (one process per window) | `surface=app&app=explorer`, `path`, `tabs`, `tabTicket` | window state + tab session, desktop drag-detach (`window.rs:597-624`) |
| win11-app `--app settings/taskmgr/alles/disks` (+defaultapps/installer) | `.Settings`/`.TaskManager`/`.Search`/`.Disks` | NON_UNIQUE; alles is HANDLES_COMMAND_LINE | `surface=app&app=<kind>` + `--params` keys | alles forwards `--params` from a second launch as `app.navigate` (`lib.rs:429-449`, `window.rs:719-734`) |
| win11-default-apps | defaultapps / `.DefaultApps` | NON_UNIQUE | `app=defaultapps` | |
| win11-installer | installer / `.Installer` | NON_UNIQUE | `app=installer` | file drop → `win11:files-dropped` |
| win11-file-portal | runtime kind `picker`, `.FilePortal` (IS_SERVICE) | D-Bus activated | base `?token=` (`portal.rs:452-455`); each request goes through `picker::create` | picker window `window::build(kind "picker")`, URL `token&surface=app&app=explorer&picker=<options JSON>&path=<dir>` (`picker.rs:89-97`), modal Dialog, transient for the external parent; at most 4 pickers |
| any app: `shell.pickFile` | same picker inside that process (`runtime.rs:156`, `window.rs:311-313`) | | | |
| win11-launch | no GTK, no WebView | | | GIO launch, Dutch errors back through the ack socket |
| win11-index/image/software-worker | no WebView | | | |

The language reaches the UI only through `settings.get`/`settings.changed` (`theme.js:319`). The cached first paint (`theme.js` `applyCached`) does not include the language. In English mode, the first paint before settings arrive therefore renders Dutch. This is minor and I did not verify it. A safe fix is a non-forcing hint, for example `window.__win11shellHost.lang` in `NATIVE_BRIDGE`, read by `i18n.js` as its starting value.

### (c) Window size memory

- **Which apps:** `eligible()` `window_state.rs:21-26` allows explorer, settings, taskmgr, alles, disks, defaultapps and installer. The extension accepts the same 7 ids (`G/extension/panelBridge.js:505,518`). Start, panel, desktop and the picker are excluded. **Third-party apps are not covered at all.** The unit test at `window_state.rs:451-465` leaves out `installer` (a gap in the test only).
- **Storage:** `$XDG_CONFIG_HOME/win11-gnome/window-state.json`, shaped `{"version":1,"apps":{kind:{normalRect:{x,y,w,h},maximized,session}}}`.
  - Written with mode 0600, an flock on `.window-state.lock`, temp file + fsync + rename, and a merge that keeps other kinds (`:186-267`). The file may be at most 8 MiB; Explorer's session at most 1 MiB, 20 tabs and 60 history entries.
  - Robin's live file (read-only) has entries for explorer, settings and taskmgr.
- **Capture:**
  - a worker thread calls `windows.hostState` on every 500 ms tick and after every command (`:313-346`, `:404-422`), which is a synchronous D-Bus call into gnome-shell;
  - extra captures are queued on every GTK `window-state-event` and on `delete-event` (`window.rs:764-808`);
  - the extension returns the frame rect plus a remembered non-maximized "normal" rect (`panelBridge.js:504-528`).
- **Write:** only in `flush()`, which runs when `app.run_with_args` returns (`lib.rs:471-473`) or on `Drop`. Nothing is written if the extension never answered (`observed == false`). There is no SIGTERM/SIGHUP handler anywhere in `host/src` (checked with grep), so a window still open at logout or shutdown loses its latest geometry.
- **Restore:**
  1. The GTK default size comes from the saved w/h (`window.rs:205-239`), with a minimum of 640×480 (`:243-258`).
  2. If the window was maximized, `window.maximize()` is called before showing it (`:809-813`).
  3. After `show_all`, `state.restore()` (`:829-831`) makes the worker call `windows.hostRestore` up to 12 times, every 500 ms (`window_state.rs:335-344`).
  4. The extension waits for the first painted frame, clamps the rect to the work area of the monitor that contains the saved top-left corner (falling back to the primary monitor), calls `move_resize_frame`, and maximizes only after the normal frame is committed (`panelBridge.js:529-565`).

  Size is therefore correct from the first frame, and the position follows after the first frame, which may cause a visible jump (not verified). Without the extension, only the size is restored, because Wayland clients cannot position themselves.
- **Per-kind, last close wins.** Each Explorer window is its own process with its own `WindowState`, and the last one to exit overwrites the entry. A newly opened second window gets exactly the same rect as the first (no cascade like Windows).
- **Missing for "remember the last window size of every app":**
  1. Third-party windows need an extension feature: track `Meta.Window` by app id (`Shell.WindowTracker`/gtk-application-id/wm-class), save the normal rect and maximized state on size change (debounced) and on `unmanaging`, and restore on `window-created` or first frame.
     - Skip dialogs, transients, fullscreen and fixed-size windows.
     - Decide between size only and size plus position.
     - Opt-outs are needed for apps that restore themselves (Firefox, Chromium, many GTK4/libadwaita apps).
     - It needs a store: a JSON file in `~/.config/win11-gnome` or a GSettings key in `extension/schemas`.
     - The entry count should be capped.
  2. A setting plus a Settings toggle with nl/en strings.
  3. For our own apps: save on change instead of only at exit, a SIGTERM handler (`glib::unix_signal_add_local` → `app.quit()`), cascading for a second instance, optionally picker size memory (Windows remembers file dialog size), and a pushed state instead of the 500 ms poll.
  4. A decision on whether own apps move onto the extension store (one mechanism) or keep `window-state.json`, which carries Explorer's tab session.

### (d) English already present

- **Settings and the provider:**
  - `settings-defaults.json:3`;
  - the settings store tests cover `language: "en"` (`settings.rs:798-1098`, in another agent's area);
  - `services/mod.rs:62-69`;
  - `Files::context()` reads `settings.json` on **every** `files.*` call (`files/mod.rs:142-157`), so a language change takes effect immediately.
- **Rust nl/en already present:** the 23 inline pairs in the table above, and `read::mime_comment` (`files/read.rs:643-700`), which picks the shared-mime-info `<comment xml:lang>` for nl, or the untranslated English comment for en.
- **App catalog:**
  - when the shell language differs from the system language, it translates `.desktop` Name/GenericName/Comment/Keywords through gettext `.mo` files from `/usr/share/locale-langpack`, `/usr/share/locale` and `/usr/local/share/locale` (`apps/catalog.rs:33-36,95-133,393-510`);
  - every 150 ms it polls the language (which reads `settings.json`) and emits `apps.changed` on a change (`apps/mod.rs:221-244`);
  - Codex validated this in both languages (`G/VALIDATION_APPS_CATALOG.md:7,24`).
- **Desktop labels:** the README notes that NL/EN covers labels such as Prullenbak/Recycle Bin (`G/README.md:27`).
- **The constraint text** "Keep original GUI and NL/EN" is in `G/../../work/native-migration-state.json:48`.
- **UI:** `vendor/ui/js/core/i18n.js` supports `nl`/`en` with `makeT`, and the per-app i18n modules exist (alles, explorer, disks, desktop strings). That is outside this area; I did not count it.
- **Probable bug** (`apps/catalog.rs:300-308`): for `lang == "en"` with `translate == true` (a Dutch system), an untranslated name falls back to `record.name`, which is GIO's name in the system locale (Dutch), not the raw `Name=`. It does not happen on this PC (`LANG=en_US.UTF-8` in the gnome-shell environment, `/etc/default/locale`). `nlNames` works correctly for nl on an English system.

## Work needed (size in agent-hours)

1. **Rust i18n core** (3-4 h): a new `host/src/i18n.rs`:
   - `include_str!` TSV catalogs in H's format (key, nl, en);
   - `tr(lang, key, params)` with one-pass `{name}` fill;
   - nl as the fallback;
   - a `Lang` taken from `Files::context().language` or `ServiceContext::language()`;
   - for `picker.rs`, `transfers.rs`, `gnome.rs`, `launch.rs`, `process_scope.rs` and `wallpaper.rs`, which have no context, a small cached reader of `settings.json` (by mtime).

   Keep failure-path texts as inline pairs (H's D14).
2. **Convert the 231 Dutch strings in this area** (6-8 h):
   - reuse H's English for the ~112 exact matches;
   - write English for ~119 new ones: picker 14, process_scope 15, transfers 13, gnome 10, wallpaper 7, ~17 archive strings, clipboard 6, launch/lib 11, the rest scattered;
   - Dutch must stay byte-identical;
   - thread `ctx`/`lang` into `io_error`, `validate_name` and `detail_error`.
3. **Tests** (2-3 h):
   - an en unit test per module;
   - a Rust "Dutch literal" lint like `H/tests/i18n` (dutch-words list, allowlist);
   - a frozen-Dutch snapshot so nl output is unchanged.
4. **Window titles** (1.5-2 h): nl/en title per kind, plus `with_document_title_changed_handler` (wry 0.55 supports it, `webkitgtk/mod.rs:465-470`) to mirror `document.title`, with the localized name as fallback, as H does. Check that taskbar previews and Task View then show "Verkenner"/"File Explorer" correctly.
5. **First-paint language hint** (0.5-1 h on the Rust side): add `window.__win11shellHost.lang` to `NATIVE_BRIDGE`, read from settings at window creation. The UI change belongs to the UI area. Never `?lang=`.
6. **English from the extension in the UI** (1-2 h): either translate the 29 `failure()` messages in `panelBridge.js`, or render them by code in Rust, in `gnome::call_api` (`gnome.rs:17-27`) and `runtime.rs:170-176`.
7. **Catalog fix for en on a Dutch system** (1 h): use the raw `Name=`/`Comment=` when no English translation exists, plus a test.
8. **Deduplicate `launcher-names.json` NL_NAMES with `catalog-data.json` nlNames** (0.5 h, optional).
9. **CLI and log strings** (0.5 h, optional): `bin/app.rs` usage, also listing defaultapps/installer. Logs may stay Dutch.
10. **`.desktop` entries with `Name[en]`/`Comment[en]`** (0.5-1 h; this is `scripts/install.py`, outside this area).
11. **Window memory for own apps** (3-4 h): a SIGTERM/SIGHUP flush handler, debounced save on change, a cascade offset for a second instance, and push instead of 500 ms polling (`size-changed`/`position-changed` from the extension, or a GTK `configure-event`).
12. **Window memory for all apps** (only if Robin decides for it; 9-13 h):
    - design 1 h;
    - extension tracking/store/restore 4-6 h;
    - opt-out list and dialog/transient rules 1 h;
    - setting + Settings UI nl/en 1-2 h;
    - test in a nested GNOME Shell 2-3 h.

## Risks

- **Fidelity:** the English must match real Windows 11 wording ("Recycle Bin", "This PC", "New Text Document.txt", "- Copy", "File Explorer"). Dutch must not change during the refactor, so a frozen-Dutch test is required.
- **Switching language at runtime:** Rust texts are rendered per call, which is correct. Messages already on screen stay in the old language. A `?lang=` URL parameter would pin a window to one language.
- **Cost:** `Files::context()` reads and parses `settings.json` on every files call, and the apps watcher does so every 150 ms. Adding language lookups elsewhere should reuse a cached, mtime-checked read, not add more disk reads.
- **Mixed language from the extension** (English, 29 messages) and from libc `strerror` in `data`. Both are visible today in Dutch mode.
- **Window titles:** changing titles affects GNOME overview text and possibly Codex's validation scripts that match titles (not checked). The extension identifies windows by app id, not title (`panelBridge.js:187`), so behaviour is safe.
- **All-app window memory:**
  - it conflicts with apps that restore themselves;
  - it can make windows jump after the first frame;
  - multi-monitor and scale changes, and Xwayland vs Wayland differences, need care;
  - it must not break Win11 snap layouts;
  - extension code runs in Robin's live gnome-shell, so it should only be tested in a nested shell, and a gnome-shell crash would take the session down.
- **Polling:** the 500 ms D-Bus polling per own-app window loads gnome-shell's main loop. Extending it to all apps by polling would be wrong.

## Not checked

- `services/**`, `settings*.rs`, `runtime.rs` beyond the relay lines, `events.rs` and `backend.rs` (other areas or out of scope); `vendor/ui` JS beyond the spots cited; the extension beyond the window-state and failure handlers; `scripts/` beyond the `.desktop` block in `install.py`.
- Runtime behaviour: no binary was started. The English window titles in taskbar previews and Task View, the position jump on restore, and the Dutch flash on first paint all come from reading the code.
- Each of the ~103 "neutral" literals was not reviewed one by one, only the ones the heuristic flagged plus spot checks. Dutch text with no marker words may be missed. Test modules are excluded from all counts.
- The H TSV matching normalizes placeholders and quotes; key names and wording were not reviewed per row.
- WebKit's own native strings (context menu, form validation) and the NucBox's locale.
- Whether the portal's English D-Bus errors are ever shown to users by calling apps.
