# UI shell side of the vendored UI in G: i18n map (read-only, 2026-11-05)

Scope: `vendor/ui/js/shell/*` (14 files), `vendor/ui/js/desktop/*` (6), `vendor/ui/js/core/*` (12), `vendor/ui/js/main.js`,
`vendor/ui/index.html`, text in `vendor/ui/css` (`content:`), text-bearing `vendor/ui/assets`.
G = `/home/robin/Documents/Codex/2026-11-01/chat/outputs/win11-gnome` (branch `claude/gnome-english-update`, clean tree,
`scripts/verify-ui.py` OK: 176 files match `vendor/ui-sha256.json`, 24 changed + 15 added against the 161-file baseline).
H = `/srv/codex-claude/claude-scratch/c2c10c4c-ae6d-4f6a-a907-342de497d54f/931db6b9-2976-4e14-8e5a-041952887025/scratch-2026-10-25-a089b6`.
Paths below are relative to `G/vendor/ui/` unless they start with `H/` or `host/`.

How the numbers were made (nothing in G or H was changed): H's own read-only tools were run with node against G and
H through symlinked roots in the scratchpad: `H/tools/i18n/scan_ui.mjs` (Dutch-word ratchet scan, 1,295-word list
`H/tests/i18n/dutch-words.txt`), `H/tools/i18n/jsdicts.mjs` (makeT/pair extraction) and `H/tools/i18n/analyze.mjs`
(the 2026-10-27 inventory classifier). SHA-256 of every file was compared with `vendor/ui-original-sha256.json`
(Codex's copy of H) and with H's snapshots `H/dist/i18n-snapshots/{E0a,E1,E-review}/ui/`. Trial 3-way merges
(`git merge-file -p`, output to the scratchpad only) were run for every file both sides changed.

## Summary

* **The shell side is already almost fully bilingual.** 17 `makeT` dictionaries hold 553 keys, and nl and en are at
  100 % parity: **0 keys miss an en value** and no dictionary is nl-only. No en value contains Dutch. Codex's new
  `core/software.js` (39 keys) is bilingual as well.
* **Hard-coded Dutch-only visible text: 10 sites in 2 files.** 9 are in `core/bridge.js` (status and connection
  errors) and 1 is in `shell/startmenu.js:132` (`'aria-label': 'Wissen'`). Every other Dutch literal has an English
  branch (a language ternary) or is a bilingual search keyword. Index.html, CSS `content:` and assets carry no text.
* **Codex's patches added no Dutch-only text in this area.** Its new taskbar search field, Start uninstall menu and
  desktop "Deze pc" handling all go through `t()`.
* **The original Codex copied is exactly H before E0a/E1.** For all 34 files, `ui-original-sha256.json` equals H's
  E1 snapshot (and H's E0a snapshot for `i18n.js` and `theme.js`). So every H change since then in this area is
  E0a/E1 i18n work (11 files). Codex changed 10 files.
* **Porting is mechanical.**
  * 4 files are straight copies from H (`core/i18n.js`, `core/format.js`, `shell/notifcenter.js`,
    `shell/quicksettings.js`).
  * 7 files changed on both sides. Six of them merge cleanly. `desktop/desktop.js` has one trivial conflict: both
    rewrote the same `onLangChange` line.
  * 3 Codex-only files need nothing (`shell/shell.js`, `main.js`, `core/icons.js`).
* **The language reaches every process through settings.json.** The Rust host does not pass `?lang=`. Each process
  (panel, Start, desktop, apps) has its own SettingsRuntime on `~/.config/win11-gnome/settings.json`. A
  `settings.set language` sends a `settings.changed` invalidation datagram to every peer, with a 150 ms disk poll as
  fallback. Each WebView's `theme.js` then calls `setLang()`, and the `onLangChange` listeners re-render.
* **Host replies follow the disk, not the caller.** `services/mod.rs:62` and `files/mod.rs:142-157` read
  settings.json on every call. Because each peer reloads before it dispatches the event, the D15 race does not arise
  in G. The Rust host ignores `X-Shell-Lang`, though, so `?lang=en` screenshots still get Dutch host texts.
* **Dutch from the Rust host still reaches English shell UI.** Examples: the notification app name `"Melding"`
  (`host/src/services/notifications.rs:361`), `"Ongeldige aanvraag."` (`host/src/runtime.rs:322`) and the error
  messages (about 261 `ApiError::new` sites in `host/src`). That is the host area's work, but the UI half (H's
  "notification" fallback) depends on it.
* **Effort for this area.** Porting is about 3.5 agent-hours. Tooling, the Dutch-unchanged baseline and the
  live-switch verification add about 8 more (details below).

## Findings

### (a) How each user-visible string is written today (G)

Counts per file. **keys** = makeT keys (nl/en), **miss-en** = keys with no en value, **NL-only** = hard-coded Dutch
visible text with no English counterpart, **ternary/pair** = Dutch literal with an English branch next to it,
**keep** = Dutch literals that are data by design (bilingual search keywords, developer messages, module ids).

| file | keys nl/en | miss-en | NL-only | ternary/pair | keep | notes |
|---|---|---|---|---|---|---|
| core/bridge.js | 0 | – | **9** | 6 | 0 | NL-only: 65 `'Verbonden'`, 66 `'Verbinding verbroken'`, 133 and 163 `'Er ging iets mis.'`, 152 offline, 155 unauthorized, 159 bad response, 169 mock failed, 238 `'Ongeldige aanvraag.'`. The launcher-trust dialog 193-209 is an `en ? … : …` ternary |
| core/components.js | 21/21 | 0 | 0 | 0 | 0 | 1426 `'aria-label': 'Breadcrumb'` is hard-coded **English** (shown to Dutch screen readers too) |
| core/software.js (Codex, new) | 39/39 | 0 | 0 | 0 | 0 | 52 formats MB with `Intl.NumberFormat(document.documentElement.lang \|\| 'en', …)`, so `'nl'`/`'en'`, not `locale()` |
| core/format.js | 0 | – | 0 | 3 | 8 | `duration()` 71-79 and `relTime()` 103-104 word ternaries, `date()` 133 nl `/`→`-` |
| core/flyouts.js | 0 | – | 0 | 0 | 1 | 35 developer error `create() ontbreekt` |
| core/i18n.js, theme.js, contextmenu.js, dom.js, charts.js, icons.js, regions.js | 0 | – | 0 | 0 | 0 | |
| desktop/strings.js | 105/105 | 0 | 0 | 0 | 0 | the desktop's single dictionary (desktop.js, menus.js, properties.js, toast.js and wallpaper.js use it) |
| desktop/desktop.js, menus.js, properties.js, toast.js, wallpaper.js | 0 | – | 0 | 0 | 0 | menus.js shortcut labels (`Ctrl+V`, `Alt+Enter`) are the same in both languages |
| main.js | 11/11 | 0 | 0 | 0 | 0 | |
| shell/clipboard.js | 18/18 | 0 | 0 | 0 | 0 | |
| shell/notifcenter.js | 19/19 | 0 | 0 | 1 | 0 | 462 weekday-header case `lang() === 'en' ? …` |
| shell/osd.js | 3/3 | 0 | 0 | 0 | 0 | |
| shell/power.js | 23/23 | 0 | 0 | 0 | 0 | |
| shell/quicksettings.js | 74/74 | 0 | 0 | 0 | 0 | the code still *reads* Dutch host text: 866 `/wachtwoord\|…/`, 928 `/reageert niet\|…/` |
| shell/search.js | 102/102 | 0 | 0 | 2 (7 values) | 63 + 17 | 689 `'Persoonlijke map'` ternary; quick chips 851-853 (6 Dutch query strings, en via `t(key).toLowerCase()` at 865); SYNONYMS 175-204 and ACTIONS `k:` 356-377 are bilingual keywords (keep). FALLBACK_SETTINGS 207-225 have nl+en titles but **nl-only keywords** (`en: []`). They are effectively dead, because `registry.js` is imported statically (20, 337) |
| shell/shell.js | 17/17 | 0 | 0 | 0 | 2 | 67 and 529 `'startmenu'` are module names (529 is Codex's) |
| shell/startmenu.js | 44/44 | 0 | **1** | 0 | 0 | 132 `'aria-label': 'Wissen'` (clear button) |
| shell/taskbar.js | 40/40 | 0 | 0 | 0 | 0 | 61 en `'Alles'` is the proper name (D5) |
| shell/taskview.js | 8/8 | 0 | 0 | 0 | 0 | |
| shell/toasts.js | 3/3 | 0 | 0 | 0 | 0 | |
| shell/tray.js | 6/6 | 0 | 0 | 0 | 0 | |
| shell/winx.js | 20/20 | 0 | 0 | 0 | 0 | 22 en `'Alles'` (D5) |
| shell/flyouthost.js | 0 | – | 0 | 0 | 0 | |
| index.html | – | – | 0 | – | – | `<html lang="nl">` (2) is overwritten by `setLang`. `<title>Win11 Shell</title>` is neutral |
| css/*.css `content:` | – | – | 0 | – | – | no non-empty `content:` strings in any stylesheet |
| assets/** | – | – | 0 | – | – | no SVG `<text>`. `wallpapers/SOURCES.json` and `icons/NOTICE.txt` are metadata, never shown |

Totals for the area: 17 dicts, 553 keys, 0 missing en, 0 nl-only dicts. Scanner hits: 127 in G (bridge 15, flyouts 1,
format 8, search 100, shell 2, startmenu 1), against 74 in H for the same files (flyouts 1, format 8, search 64,
shell 1). The 74 are all `keep` entries in `H/tests/i18n/allow-ui.txt`. That allowlist has 90 lines in all, of
which 72 cover these files.

### (b) Choosing and propagating the language

* **UI runtime (G = H before E0a).**
  * `core/i18n.js` starts as `'nl'`. It reads `?lang=` once at module load (15). `locale()` returns `'en-GB'` or
    `'nl-NL'` (22). `setLang()` (24-29) sets `document.documentElement.lang` and calls the listeners.
  * Plural choice is `Number(params?.n ?? params?.count ?? 0)` (53): this is the pre-E0a code with the
    "formatted n" flaw. No caller in this area passes a formatted n: `taskbar.js:298` and `:736`,
    `search.js:386`, `desktop.js:458` and `:475`, and `properties.js:236` all pass raw numbers, so nothing is
    wrong on screen here.
  * `core/i18n.js` has no `list()`, `collator()`, `countOf()` or `i18nReport`.
* **theme.js.**
  * `FALLBACK.language = 'nl'` (27). `forcedLang` comes from `?lang=` and wins (50).
  * `apply()` runs `setLang(forcedLang || s?.language || 'nl')` (319-321) after `settings.get` in `initTheme()`
    (361-369) and on every `settings.changed` (371-379).
  * `main.js` awaits `initTheme()` before it mounts a surface (74), so no Dutch text flashes for an English user.
  * The localStorage theme cache stores no language. That is H's E0a addition, and H's `theme.js` still equals its
    E0a snapshot, so E1 did not touch the file.
* **What the Rust host gives a WebView.**
  * The URL is `http://127.0.0.1:{port}/ui/index.html?token=…&surface=shell|desktop|app[&app=<kind>]`
    (`host/src/lib.rs:373-386`). `portal.rs:451` and `picker.rs:91` build their URLs the same way.
  * The init scripts set `window.__win11shellHost` = `{backdrop, runtime:'rust', windowId, preloadStart,
    explorerSession, nativeBackdrop, standaloneStart}` (`host/src/lib.rs:138-142`, `window.rs:482`).
  * **No language, no `?lang=`.** The language comes only from `settings.get`.
* **Separate processes and a live switch.**
  * Each host builds its own `SettingsRuntime` on `$WIN11_GNOME_CONFIG_DIR` or `~/.config/win11-gnome`
    (`host/src/settings_runtime.rs:40-42`).
  * A `settings.set` emits `settings.changed` locally and then sends `{_settingsInvalidation:true, path}` to its
    peers over a Unix datagram (`events.rs:160-169`).
  * Each peer's receive thread calls the settings loader: `adopt_disk_exact()` reads the file again, then dispatches
    the full settings to its SSE clients (`events.rs:105-124`, `settings_runtime.rs:65-73`).
  * A 150 ms `store.reload()` poll backs this up (`settings_runtime.rs:75-89`).
  * The apps catalogue also polls the language every 150 ms and emits `apps.changed` when it changes
    (`host/src/apps/mod.rs:221-244`). So Start, taskbar and search fetch the app names again (`Name[en]`).
  * A switch made in Settings therefore reaches panel, Start, desktop, Explorer and any other host within about
    150 ms.
* **Host-side text language.**
  * `ctx.language()` reads `store.snapshot()` from disk on each call (`services/mod.rs:62-69`, `settings.rs:77-80`).
  * `files::context()` reads settings.json on each call (`files/mod.rs:142-157`).
  * Every peer has re-read the file before it dispatches the event, so replies to the refetch are already in the
    new language. H needed `X-Shell-Lang` to avoid this race (D15); G does not.
  * G's `bridge.js:143-146` sends only `X-Shell-Token`. The only header the Rust server checks is the token
    (`runtime.rs:254-266`). All UI fetches are same-origin (`/api/call/…`, `bridge.js:143`,
    `apps/taskmgr/hostcall.js:95`), so adding the header triggers no CORS preflight.
* **Which surfaces re-render on a switch** (`onLangChange` in G).
  * They are: clipboard 94, winx 172, notifcenter 122, power 94, tray 137, quicksettings 191 (rebuilds the UI but
    does not re-fetch host text), startmenu 709, taskview 197, taskbar 1051 (also rebuilds Codex's search input),
    search 910, desktop 1117 (Codex: `reloadSoon()`, which also re-fetches host labels), and software 97/157.
  * Gaps that H's E1 fixed and G still has: the region `aria-label` in `toasts.js:59` is built once; the
    startmenu `aria-label`s on the input and the clear button are not refreshed; quick settings does not run
    `refreshAll()` again, so device and network texts stay in the old language until the next poll.
* **Theme cache and per-process ports.** Each process binds `127.0.0.1:0` (`runtime.rs:203`), so every launch gets
  a new origin and the localStorage `win11shell.themeCache` starts empty. Porting E0a's language-in-cache is
  harmless, but in G it rarely helps: the "settings.get failed" path will still fall back to `'nl'`.

### (c) Dates, numbers, plurals, sorting

* **Numbers.** `core/format.js` `nf()` caches `Intl.NumberFormat(locale())` (8-10). `bytes`, `number`, `percent`
  and bitrate go through it. The exception is `core/software.js:52`, which uses a bare `'nl'`/`'en'` and its own
  unit formatting. Its output is the same.
* **Dates and times.**
  * `date()` and `time()` build a new `Intl.DateTimeFormat(locale())` on every call (132, 142). H caches them
    (`dtf()`/`rtf()`, E1).
  * Short date: nl `25-10-2026` (`/`→`-` at 133), en-GB `25/10/2026`.
  * The taskbar clock uses `fmtTime`, `fmtDate(now,'short')` and a long-date tooltip (`taskbar.js:705-712`).
    24-hour time comes from `settings.time.h24`, not from the language.
  * The calendar in `notifcenter.js:451-470` uses `Intl` with `locale()`. Its weekday header is 2 letters (lower
    case in nl). The first day of the week is `settings.time.firstDayOfWeek`, default Monday (402-405), in both
    languages.
  * `relTime()`: nl `'zojuist'`/en `'just now'` under 45 s, otherwise `Intl.RelativeTimeFormat` (103-111).
  * `whenLabel()` (`startmenu.js:64-76`) uses `t()` together with format.js.
  * `duration()` words are a nl/en ternary (H keeps it the same way).
* **Plurals.** All plural objects in this area are `{one, other}`: `desktop/strings.js` 57/60/71/123 (+en),
  `search.js:35/75` and `taskbar.js:38/43/54/59`. Every caller passes a raw count. The nl and en rule is "one iff
  1", which is correct.
* **Sorting.** None of the sorting sites goes through a per-language collator, but none shows a visible difference
  either, because Dutch has no CLDR tailoring:
  * `startmenu.js:61` `Intl.Collator('nl', {sensitivity:'base', numeric:true})`, used at 378;
  * `desktop.js:38` `Intl.Collator(undefined, {numeric:true, sensitivity:'base'})`, which takes the system
    default locale;
  * bare `localeCompare()` at `search.js:518` and `quicksettings.js:1029`.

  H's E1 moved all of these to `collator(opts)` and kept each site's own options.

### (d) G against H, per file

Base = the file Codex copied (= H's E1 snapshot; = H's E0a snapshot for i18n.js and theme.js). "Codex" means
G ≠ base; "H" means H ≠ base. Every H change listed here is E0a, E1 or E1-review i18n work. The diffs were read;
none is a non-i18n change.

| file | Codex changed | H changed | trial 3-way merge | what H adds |
|---|---|---|---|---|
| core/i18n.js | no | yes (E0a, 65→153 lines) | copy | `countOf()`, `i18nReport` (`?i18ncheck=1`), `list()`, `collator()` |
| core/theme.js | yes (native backdrop, +48 lines at 73-118, 307) | yes (E0a, 2 hunks: `lang` in the cache, cache fallback) | clean | language in the theme cache (of little use in G, see (b)) |
| core/bridge.js | yes (1 line 273: settings icon URL) | yes (E1, 23 hunks) | clean | makeT with 14 keys (all 10 NL-only sites), `X-Shell-Lang`, `e.key`/`e.params`, status message rendered in the current language, `demoLauncherQuestion()` |
| core/format.js | no | yes (E1) | copy | cached `dtf()`/`rtf()` |
| core/icons.js | yes (settings.svg) | no | keep G | – |
| core/software.js | new (Codex) | absent | keep G | – |
| other core (charts, components, contextmenu, dom, flyouts, regions) | no | no | – | – |
| shell/search.js | yes (taskbar-search commands, `onQuery`, keys `web:`/`alles:`+query, `onLangChange` also rebuilds the settings index, `focus:false`) | yes (E1: `qq_*` and `homeFolder` keys, FALLBACK_SETTINGS as `{nl,en}`, collator, en `'Task view'`) | clean | 7 keys, the 2 ternaries removed |
| shell/startmenu.js | yes (software removal menu, entrance clip for standalone Start, recAll clearing) | yes (E1: `clear` key, collator, aria-labels on switch) | clean | fixes the NL-only `'Wissen'` |
| shell/taskbar.js | yes (real search input, centring) | yes (en `'Task View'`→`'Task view'`) | clean | 1 en string |
| shell/toasts.js | yes (GNOME silent, replace by id) | yes (E1: `notification` key, aria-label on switch) | clean | needs the host to store `""` for a missing app name |
| shell/notifcenter.js | no | yes (E1: `notification` fallback) | copy | same host dependency |
| shell/quicksettings.js | no | yes (E1 + E1 review: collator, `refreshAll()` on switch, Wi-Fi `bad_args` with a password, BT by code or key) | copy | Rust already returns the codes it needs: `not_ready` (`services/system/bluetooth.rs:340,1261`), `bad_args` "Ongeldig wachtwoord." (`services/system/network.rs:336`) |
| shell/shell.js | yes (standalone Start: no taskbar, 3 modules) | no | keep G | – |
| other shell (clipboard, flyouthost, osd, power, taskview, tray, winx) | no | no | – | – |
| desktop/desktop.js | yes (`labelFor` thispc, hide the duplicate Deze-pc file, `onLangChange`→`reloadSoon()`) | yes (E1: collator, `onLangChange`→`refreshView(); reload()`, `?open=launcher-trust` demo) | **1 conflict** at the `onLangChange` (G 1117-1121). Resolve to `{ if (loaded) { refreshView(); reloadSoon(); } }` | collator, the demo case (it needs H's bridge.js) |
| other desktop (menus, properties, strings, toast, wallpaper) | no | no | – | – |
| main.js | yes (`APPS` adds defaultapps, installer) | no | keep G | – |
| index.html | no | no | – | – |

H file versions read (sha256 prefix and mtime), so a later port can tell whether H moved:

* `i18n.js` 7876deb43c0a (11-04 14:10)
* `theme.js` dac7e3bf5732 (11-04 14:10)
* `bridge.js` 9088f0700331 (11-05 07:32)
* `format.js` 9db95278a5db (07:33)
* `search.js` 1b63ee299f96 (07:52)
* `startmenu.js` cb306f145984 (07:38)
* `taskbar.js` a2fe8e0baa61 (07:52)
* `toasts.js` bfb21b64da41 (08:02)
* `notifcenter.js` 694a605aad21 (08:01)
* `quicksettings.js` b24e1f788022 (09:41)
* `desktop.js` b3f94cc0cdec (07:46)

## Work needed (this area; agent-hours)

1. **Record the Dutch baseline first.** Write a text dump of every shell surface in G in nl before touching
   anything. Use the UI's mock mode (`?mock=1`) from a static server on `vendor/ui`, following
   `H/tests/i18n/dump.html` and `serve.py`. Note that Codex changed `mock/system.js`. Run it in a private or nested
   session only. **3 h**
2. **Copy the 4 untouched files from H:** `core/i18n.js`, `core/format.js`, `shell/notifcenter.js`,
   `shell/quicksettings.js`. **0.5 h**
3. **3-way merge the 6 clean files** (`theme.js`, `bridge.js`, `search.js`, `startmenu.js`, `taskbar.js`,
   `toasts.js`). Review each one so Codex's behaviour stays, in particular `search.js`'s `onLangChange` with
   `buildSettingsIndex()`. **1.5 h**
4. **Merge `desktop/desktop.js` and resolve its one conflict.** Keep Codex's debounced `reloadSoon()` and add
   `refreshView()`. **0.25 h**
5. **"Melding" end to end.**
   * The UI half comes with items 2 and 3.
   * The host half (host area): `services/notifications.rs:361` should store `""`, and stored or history items with
     `"Melding"` should be mapped to `""`. Then check the GNOME-source notifications.
   * **0.25 h UI + 0.5 h host**
6. **Optional: `components.js:1426` `'Breadcrumb'`.** Put it in a dictionary with nl `'Breadcrumb'`, so Dutch output
   stays unchanged, or decide on a Dutch word. That would be a recorded Dutch change. **0.1 h**
7. **Bookkeeping for the vendored UI.** Regenerate `vendor/ui-sha256.json`, add an `english_option` entry to
   `vendor/ui-patches.json`, and run `scripts/verify-ui.py`, which `scripts/package.py:17` runs as a gate.
   **0.25 h**
8. **Bring H's i18n tooling to G.**
   * Tools: `tools/i18n/{tok,jsdicts,scan_ui,pairs}.mjs`, the UI part of `check.py`, `ratchet.py`,
     `dutch-words.txt` and `allow-ui.txt`.
   * They hardcode `ROOT/ui/js`; G has `vendor/ui/js`.
   * Add allow entries for G-only keeps (`shell.js:529`).
   * **1.5 h**
9. **Check after the port.**
   * nl dump against the baseline from item 1 (byte-identical, apart from intended changes).
   * en dump with `?i18ncheck=1`: `window.__i18n` must report nothing missing.
   * Ratchet scan: expect 74 keeps plus G-only keeps.
   * **1.5 h**
10. **Live switch across the separate processes, in a nested GNOME session (never the live display).**
    * Switch in Settings, then check panel, Start (also preloaded and hidden), desktop, Explorer and Settings:
      `html[lang]`, labels, app names after `apps.changed`, and quick settings texts after `refreshAll()`.
    * Check the English layout for truncation (taskbar search field, Start, quick settings).
    * **2.5 h**
11. **Optional, host area: make the Rust host read `X-Shell-Lang` per request.** Without it, `?lang=en` screenshots
    against a Dutch settings.json show Dutch host texts. It is not needed for normal use. **1–2 h**

Total for this area: about **3.5 h of porting** (items 2-7) plus about **8.5 h of tooling and verification**
(items 1, 8-10). Host translations (about 261 `ApiError::new` sites, `"Ongeldige aanvraag."`, etc.) are not
included.

## Risks

* **Dutch must not change.**
  * The port changes no Dutch-visible text, unless item 6 is done with a Dutch word.
  * H's E0a `countOf` change only differs for string `n`, and no caller here passes one.
  * Dropping the Dutch text matches in quick settings relies on the host codes above. Rust's messages are still
    Dutch, so the English UI shows Dutch host errors until the host area translates them.
* **Codex's manifest gate.** `verify-ui.py` fails packaging unless `ui-sha256.json` is regenerated. Other Codex
  checks may also compare against `ui-original-sha256.json`; not checked.
* **desktop.js conflict semantics.** Calling both `reload()` and `reloadSoon()` would fetch twice. Keep only the
  debounced one. Codex also hides the duplicate "Deze pc" file in the view, so make sure its `t('thispc')` label
  follows the switch (`labelFor`, desktop.js:109-110).
* **Module order.** H's `bridge.js` imports `i18n.js` statically. H saw this shift mock timer numbers in dumps.
  It is harmless at runtime, but record the nl baseline with the same module graph, or compare in a way that
  ignores mock numbers.
* **The hidden preloaded Start** (`lib.rs:211-217`) may throttle timers and requestAnimationFrame. A language switch
  while it is hidden must still re-render before the next open. This is unverified.
* **H may still move.** The memory notes an "English round 2" running in H. Port from the hashes listed above and
  re-check before merging.
* **Fidelity.** English strings are longer. Codex's real taskbar search `<input>` with the placeholder "Search" and
  Start's English labels need a visual check against Windows 11 en-GB.

## Not checked

* **No runtime checks.** Nothing was launched and no screenshots were taken. The live-switch behaviour across
  processes, the hidden-Start behaviour and the SSE delivery were read from code only.
* **Outside this area:** `js/apps/**` (Settings, including the language picker in `apps/settings/registry.js:139`
  and `pages/time.js`; Explorer, Taakbeheer, Alles, Schijven, defaultapps, installer), `js/mock/**`,
  `js/preview/**`, `gallery.js`, `extension/`, and the Rust host's own texts. I only spot-checked those that feed
  the shell: notifications, the bluetooth and Wi-Fi codes, and `runtime.rs` errors.
* **Scan coverage.** The Dutch detection is word-list based (1,295 words plus the analyzer's vocabulary). A Dutch
  string with none of those words would be missed. I added a manual check of literal `aria-label`, `title`,
  `placeholder` and `tooltip` values and a review of every "neutral" literal in the area, and found no further
  Dutch.
* **English quality.** The en text was not checked against H's glossary (`H/tools/i18n/glossary.tsv`) beyond the two
  E1 changes ("Task view").
* **Other code and tests.** I did not check whether other Codex scripts or tests pin UI file hashes besides
  `verify-ui.py`, `install.py:72` and `package.py:17`. I did not run H's `tests/i18n/core_i18n.test.mjs` against
  G.
* **Trial merges.** The 3-way merges exist only in the scratchpad
  (`/tmp/claude-1000/-srv-codex-claude-claude-scratch-c2c10c4c-ae6d-4f6a-a907-342de497d54f-931db6b9-2976-4e14-8e5a-041952887025-scratch-2026-10-25-a089b6/36a3431f-8624-40a3-90db-1ea7dd8f4bf5/scratchpad/uishell/merge/`).
  They have not been reviewed line by line beyond the conflict.
